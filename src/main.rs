#![windows_subsystem = "windows"]
use anyhow::{Context, Result};
use miclift::{
    audio::{Devices, Engine},
    config::Config,
    dsp, platform,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::atomic::Ordering,
    time::Duration,
};
slint::include_modules!();

struct App {
    ui: slint::Weak<MainWindow>,
    config: RefCell<Config>,
    devices: RefCell<Devices>,
    engine: Engine,
    meters: Timer,
    save_timer: Timer,
    dirty: Cell<bool>,
    preview: bool,
    bands_model: Rc<VecModel<Band>>,
}
impl App {
    fn notice(&self, message: impl Into<SharedString>) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_notice(message.into());
        }
    }
    fn persist(&self) {
        if self.preview || !self.dirty.replace(false) {
            return;
        }
        let mut config = self.config.borrow_mut();
        if !config.input_id.is_empty() {
            let id = config.input_id.clone();
            let voice = config.voice;
            config.profiles.insert(id, voice);
        }
        if let Err(error) = config.save() {
            self.dirty.set(true);
            self.notice(format!("Settings could not be saved: {error:#}"));
        }
    }
    fn changed(self: &Rc<Self>) {
        let config = self.config.borrow();
        self.engine.parameters.store(&config.voice);
        self.engine
            .parameters
            .monitor_volume
            .store(config.monitor_volume.to_bits(), Ordering::Relaxed);
        drop(config);
        self.sync_settings();
        self.dirty.set(true);
        let weak = Rc::downgrade(self);
        self.save_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(500),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.persist();
                }
            },
        );
    }
    fn sync_settings(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let config = self.config.borrow();
        let voice = config.voice;
        ui.set_boost(voice.boost);
        ui.set_listen_volume(config.monitor_volume * 100.0);
        ui.set_eq_enabled(voice.eq_enabled);
        ui.set_high_pass(voice.high_pass);
        ui.set_gate(voice.gate);
        ui.set_gate_threshold(voice.gate_threshold_db);
        ui.set_compressor(voice.compressor);
        ui.set_compressor_threshold(voice.compressor_threshold_db);
        ui.set_compressor_ratio(voice.compressor_ratio);
        ui.set_limiter(voice.limiter);
        ui.set_start_windows(config.start_with_windows);
        ui.set_start_minimized(config.start_minimized);
        ui.set_start_audio(config.start_audio);
        ui.set_close_tray(config.close_to_tray);
        for (index, band) in voice.bands.iter().enumerate() {
            self.bands_model.set_row_data(
                index,
                Band {
                    frequency: band.frequency,
                    gain: band.gain_db,
                    q: band.q,
                },
            );
        }
        let mut path = String::new();
        use std::fmt::Write;
        for index in 0..=120 {
            let frequency = 20.0 * 1000.0_f32.powf(index as f32 / 120.0);
            let db = dsp::response_db(&voice, 48000.0, frequency).clamp(-18.0, 18.0);
            let _ = write!(
                path,
                "{} {:.2} {:.2} ",
                if index == 0 { "M" } else { "L" },
                index as f32 * 700.0 / 120.0,
                60.0 - db * 3.0
            );
        }
        ui.set_eq_curve(path.into());
    }
    fn sync_devices(&self, devices: Devices) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let filtered = Devices {
            inputs: devices.inputs.into_iter().filter(|d| !d.is_cable).collect(),
            outputs: devices.outputs,
        };
        let mut config = self.config.borrow_mut();
        let inputs = &filtered.inputs;
        let outputs: Vec<_> = filtered.outputs.iter().filter(|d| !d.is_cable).collect();
        let cables: Vec<_> = filtered.outputs.iter().filter(|d| d.is_cable).collect();
        // Only resolve defaults on first configuration. Missing saved IDs are retained for reconnect.
        if config.input_id.is_empty()
            && let Some(device) = inputs
                .iter()
                .find(|d| d.is_default)
                .or_else(|| inputs.first())
        {
            config.input_id = device.id.clone();
        }
        if config.monitor_id.is_empty()
            && let Some(device) = outputs
                .iter()
                .find(|d| d.is_default)
                .or_else(|| outputs.first())
        {
            config.monitor_id = device.id.clone();
        }
        if config.cable_id.is_empty()
            && let Some(device) = cables.first()
        {
            config.cable_id = device.id.clone();
        }
        ui.set_input_names(ModelRc::new(VecModel::from(
            inputs
                .iter()
                .map(|d| SharedString::from(d.name.as_str()))
                .collect::<Vec<_>>(),
        )));
        ui.set_output_names(ModelRc::new(VecModel::from(
            outputs
                .iter()
                .map(|d| SharedString::from(d.name.as_str()))
                .collect::<Vec<_>>(),
        )));
        ui.set_cable_names(ModelRc::new(VecModel::from(
            cables
                .iter()
                .map(|d| SharedString::from(d.name.as_str()))
                .collect::<Vec<_>>(),
        )));
        ui.set_input_index(
            inputs
                .iter()
                .position(|d| d.id == config.input_id)
                .map_or(-1, |i| i as i32),
        );
        ui.set_output_index(
            outputs
                .iter()
                .position(|d| d.id == config.monitor_id)
                .map_or(-1, |i| i as i32),
        );
        ui.set_cable_index(
            cables
                .iter()
                .position(|d| d.id == config.cable_id)
                .map_or(-1, |i| i as i32),
        );
        *self.devices.borrow_mut() = filtered;
    }
    fn tick(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if !ui.window().is_visible() || ui.window().is_minimized() {
            return;
        }
        let state = self.engine.status.lock().unwrap().clone();
        if *self.devices.borrow()
            != (Devices {
                inputs: state
                    .devices
                    .inputs
                    .iter()
                    .filter(|d| !d.is_cable)
                    .cloned()
                    .collect(),
                outputs: state.devices.outputs.clone(),
            })
        {
            self.sync_devices(state.devices.clone());
        }
        ui.set_running(state.running);
        ui.set_requested(state.requested);
        ui.set_status_text(state.message.into());
        ui.set_muted(self.engine.parameters.muted.load(Ordering::Relaxed));
        ui.set_listening(self.engine.parameters.monitor.load(Ordering::Relaxed));
        let telemetry = &self.engine.telemetry;
        let input = f32::from_bits(telemetry.input_peak.swap(0, Ordering::Relaxed));
        let output = f32::from_bits(telemetry.output_peak.swap(0, Ordering::Relaxed));
        let db = |value: f32| {
            if value < 0.00001 {
                "−∞ dBFS".to_string()
            } else {
                format!("{:.1} dBFS", 20.0 * value.log10())
            }
        };
        ui.set_input_db(db(input).into());
        ui.set_output_db(db(output).into());
        let meter = |value: f32| {
            if value < 0.00001 {
                0.0
            } else {
                ((20.0 * value.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
            }
        };
        ui.set_input_level(meter(input).max(ui.get_input_level() * 0.90));
        ui.set_output_level(meter(output).max(ui.get_output_level() * 0.90));
        if state.running {
            let queue_ms = telemetry.cable_queue.load(Ordering::Relaxed) as f32 * 1000.0
                / state.input_rate.max(1) as f32;
            ui.set_timing_text(format!("Device periods: mic {:.1} ms · cable {:.1} ms  |  Queue {:.1} ms (not total latency)", telemetry.capture_period_us.load(Ordering::Relaxed) as f32 / 1000.0, telemetry.cable_period_us.load(Ordering::Relaxed) as f32 / 1000.0, queue_ms).into());
        } else {
            ui.set_timing_text(
                "Low-latency WASAPI · shared mode · live monitoring without recording".into(),
            );
        }
        ui.set_detail_text(format!("DSP p99 ≤ {} µs/block · cable underruns {} · monitor underruns {} · overrun frames {}", telemetry.dsp_p99_us(), telemetry.cable_underruns.load(Ordering::Relaxed), telemetry.monitor_underruns.load(Ordering::Relaxed), telemetry.overruns.load(Ordering::Relaxed)).into());
    }
    fn show(self: &Rc<Self>) {
        if let Some(ui) = self.ui.upgrade() {
            let _ = ui.show();
            ui.window().set_minimized(false);
        }
        self.tick();
        let weak = Rc::downgrade(self);
        self.meters
            .start(TimerMode::Repeated, Duration::from_millis(34), move || {
                if let Some(app) = weak.upgrade() {
                    app.tick();
                }
            });
    }
    fn hide(&self) {
        self.persist();
        self.meters.stop();
        if let Some(ui) = self.ui.upgrade() {
            let _ = ui.hide();
        }
    }
    fn toggle_audio(&self) {
        let requested = self.engine.status.lock().unwrap().requested;
        if requested {
            self.engine.stop();
        } else {
            let mut config = self.config.borrow_mut();
            config.setup_complete = true;
            self.dirty.set(true);
            self.engine.start(&config);
        }
    }
    fn restart_if_active(&self) {
        if self.engine.status.lock().unwrap().requested {
            self.engine.start(&self.config.borrow());
        }
    }
    fn mute(&self) {
        self.engine
            .parameters
            .muted
            .fetch_xor(true, Ordering::Relaxed);
        self.tick();
    }
    fn close(&self) {
        if self.config.borrow().close_to_tray {
            self.hide();
        } else {
            self.persist();
            let _ = slint::quit_event_loop();
        }
    }
}

fn connect(ui: &MainWindow, tray: &Tray, app: &Rc<App>) {
    macro_rules! callback { ($method:ident, |$app:ident $(, $arg:ident)*| $body:block) => {{
        let weak: Weak<App> = Rc::downgrade(app);
        ui.$method(move |$($arg),*| { if let Some($app) = weak.upgrade() { $body } });
    }} }
    callback!(on_gain_changed, |app, value| {
        app.config.borrow_mut().voice.boost = value;
        app.changed();
    });
    callback!(on_change_band, |app, index, field, value| {
        if let Some(band) = app.config.borrow_mut().voice.bands.get_mut(index as usize) {
            match field {
                0 => band.frequency = value,
                1 => band.gain_db = value,
                _ => band.q = value,
            }
        }
        app.changed();
    });
    callback!(on_processing, |app, field, value| {
        {
            let mut config = app.config.borrow_mut();
            let voice = &mut config.voice;
            match field {
                0 => voice.eq_enabled = value > 0.5,
                1 => voice.high_pass = value > 0.5,
                2 => voice.gate = value > 0.5,
                3 => voice.gate_threshold_db = value,
                4 => voice.compressor = value > 0.5,
                5 => voice.compressor_threshold_db = value,
                6 => voice.compressor_ratio = value,
                7 => voice.limiter = value > 0.5,
                _ => {}
            }
        }
        app.changed();
    });
    callback!(on_preset, |app, name| {
        app.config.borrow_mut().voice.preset(name.as_str());
        app.changed();
    });
    callback!(on_select_input, |app, index| {
        let id = app
            .devices
            .borrow()
            .inputs
            .get(index as usize)
            .map(|d| d.id.clone());
        if let Some(id) = id {
            app.config.borrow_mut().select_input(id);
            app.changed();
            app.restart_if_active();
        }
    });
    callback!(on_select_cable, |app, index| {
        let id = app
            .devices
            .borrow()
            .outputs
            .iter()
            .filter(|d| d.is_cable)
            .nth(index as usize)
            .map(|d| d.id.clone());
        if let Some(id) = id {
            app.config.borrow_mut().cable_id = id;
            app.changed();
            app.restart_if_active();
        }
    });
    callback!(on_select_output, |app, index| {
        let id = app
            .devices
            .borrow()
            .outputs
            .iter()
            .filter(|d| !d.is_cable)
            .nth(index as usize)
            .map(|d| d.id.clone());
        if let Some(id) = id {
            app.config.borrow_mut().monitor_id = id;
            app.changed();
            app.restart_if_active();
        }
    });
    callback!(on_preference, |app, field, value| {
        if field == 0
            && let Err(error) = platform::set_autostart(value)
        {
            app.notice(format!("Startup could not be changed: {error:#}"));
            app.sync_settings();
            return;
        }
        {
            let mut config = app.config.borrow_mut();
            match field {
                0 => config.start_with_windows = value,
                1 => config.start_minimized = value,
                2 => config.start_audio = value,
                3 => config.close_to_tray = value,
                _ => {}
            }
        }
        app.changed();
    });
    callback!(on_toggle_audio, |app| {
        app.toggle_audio();
    });
    callback!(on_toggle_mute, |app| {
        app.mute();
    });
    callback!(on_toggle_listen, |app| {
        let enabled = !app.engine.parameters.monitor.load(Ordering::Relaxed);
        app.engine
            .parameters
            .monitor
            .store(enabled, Ordering::Relaxed);
        app.restart_if_active();
        app.tick();
    });
    callback!(on_volume_changed, |app, value| {
        app.config.borrow_mut().monitor_volume = value / 100.0;
        app.changed();
    });
    callback!(on_minimize, |app| {
        app.hide();
    });
    callback!(on_close_window, |app| {
        app.close();
    });
    callback!(on_show_from_instance, |app| {
        app.show();
    });
    callback!(on_setup, |app| {
        let weak = app.ui.clone();
        if let Some(ui) = weak.upgrade() {
            ui.set_setup_busy(true);
        }
        std::thread::spawn(move || {
            let result = platform::driver_setup();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_setup_busy(false);
                ui.set_notice(
                    match result {
                        Ok(message) => message,
                        Err(error) => format!("{error:#}"),
                    }
                    .into(),
                );
            });
        });
    });
    let weak = Rc::downgrade(app);
    ui.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade() {
            app.close();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = Rc::downgrade(app);
    tray.on_show_window(move || {
        if let Some(app) = weak.upgrade() {
            app.show();
        }
    });
    let weak = Rc::downgrade(app);
    tray.on_toggle_audio(move || {
        if let Some(app) = weak.upgrade() {
            app.toggle_audio();
        }
    });
    let weak = Rc::downgrade(app);
    tray.on_toggle_mute(move || {
        if let Some(app) = weak.upgrade() {
            app.mute();
        }
    });
    let weak = Rc::downgrade(app);
    tray.on_quit(move || {
        if let Some(app) = weak.upgrade() {
            app.persist();
        }
        let _ = slint::quit_event_loop();
    });
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--self-test") {
        return miclift::self_test();
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("MicLift {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "MicLift [--autostart] [--minimized] [--self-test] [--version]\nDiagnostics: miclift-diagnostics --help"
        );
        return Ok(());
    }
    let screenshot = args
        .iter()
        .position(|arg| arg == "--screenshot")
        .and_then(|index| args.get(index + 1))
        .cloned();
    let ui_test = args.iter().any(|arg| arg == "--ui-self-test");
    let benchmark = args
        .iter()
        .position(|arg| arg == "--benchmark-seconds")
        .map(|index| -> Result<u64> {
            let seconds: u64 = args
                .get(index + 1)
                .context("Missing benchmark duration")?
                .parse()?;
            anyhow::ensure!(
                (5..=86400).contains(&seconds),
                "Benchmark duration must be 5..86400 seconds"
            );
            Ok(seconds)
        })
        .transpose()?;
    let benchmark_report = args
        .iter()
        .position(|arg| arg == "--report")
        .and_then(|index| args.get(index + 1))
        .cloned();
    anyhow::ensure!(
        benchmark.is_none() || benchmark_report.is_some(),
        "Benchmark requires --report <JSON-path>"
    );
    let mut instance = if screenshot.is_none() && !ui_test {
        platform::Instance::acquire()?
    } else {
        None
    };
    if instance.is_none() && screenshot.is_none() && !ui_test {
        return Ok(());
    }
    let mut config = if screenshot.is_some() || benchmark.is_some() || ui_test {
        Config::default()
    } else {
        Config::load()?
    };
    if screenshot.is_none() && !ui_test && benchmark.is_none() {
        config.start_with_windows = platform::autostart_enabled();
    }
    let first_run = !config.setup_complete;
    let hidden =
        !first_run && (config.start_minimized || args.iter().any(|arg| arg == "--minimized"));
    let auto_audio =
        benchmark.is_some() || (config.start_audio && !first_run && screenshot.is_none());
    let ui = MainWindow::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let tray = Tray::new()?;
    if screenshot.is_some() || ui_test {
        tray.hide()?;
    }
    if cfg!(feature = "licensed-driver-setup") {
        ui.set_setup_label("Install official VB-CABLE driver".into());
    }
    let app = Rc::new(App {
        ui: ui.as_weak(),
        config: RefCell::new(config),
        devices: RefCell::new(Devices::default()),
        engine: Engine::new(),
        meters: Timer::default(),
        save_timer: Timer::default(),
        dirty: Cell::new(false),
        preview: screenshot.is_some() || benchmark.is_some() || ui_test,
        bands_model: Rc::new(VecModel::from(vec![Band::default(); 5])),
    });
    app.sync_devices(app.engine.status.lock().unwrap().devices.clone());
    app.engine.parameters.store(&app.config.borrow().voice);
    app.engine.parameters.monitor_volume.store(
        app.config.borrow().monitor_volume.to_bits(),
        Ordering::Relaxed,
    );
    ui.set_bands(app.bands_model.clone().into());
    app.sync_settings();
    connect(&ui, &tray, &app);
    if benchmark.is_some() && args.iter().any(|arg| arg == "--monitor") {
        app.engine.parameters.monitor.store(true, Ordering::Relaxed);
    }
    if let Some(instance) = &mut instance {
        let weak = ui.as_weak();
        instance.on_show(move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                ui.invoke_show_from_instance();
            });
        });
    }
    if auto_audio {
        app.engine.start(&app.config.borrow());
    }
    if !hidden {
        app.show();
    }
    if ui_test {
        let weak = Rc::downgrade(&app);
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            let result = (|| -> Result<()> {
                let app = weak.upgrade().context("App unexpectedly dropped")?;
                let ui = app.ui.upgrade().context("UI unexpectedly dropped")?;
                ui.invoke_gain_changed(500.0);
                anyhow::ensure!(ui.get_boost() == 500.0, "Gain callback failed");
                ui.invoke_change_band(2, 1, 6.0);
                anyhow::ensure!(
                    app.bands_model.row_data(2).unwrap().gain == 6.0,
                    "EQ callback failed"
                );
                ui.invoke_processing(2, 1.0);
                anyhow::ensure!(app.config.borrow().voice.gate, "Gate callback failed");
                ui.invoke_preset("Warm".into());
                anyhow::ensure!(
                    app.config.borrow().voice.bands[0].gain_db == 3.0,
                    "Preset callback failed"
                );
                ui.invoke_volume_changed(25.0);
                anyhow::ensure!(
                    app.config.borrow().monitor_volume == 0.25,
                    "Monitor volume callback failed"
                );
                // These preferences do not touch the registry. Preview mode never saves profiles.
                for index in 1..=3 {
                    ui.invoke_preference(index, true);
                }
                let inputs_available = app.devices.borrow().inputs.len();
                if inputs_available > 0 {
                    ui.invoke_select_input(0);
                }
                app.hide();
                anyhow::ensure!(!app.meters.running(), "Meter timer did not stop in tray");
                app.show();
                anyhow::ensure!(app.meters.running(), "Meter timer did not restart");
                app.close();
                anyhow::ensure!(!app.meters.running(), "Close-to-tray did not stop meters");
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("UI self-test failed: {error:#}");
                std::process::exit(1);
            }
            let _ = slint::quit_event_loop();
        });
    }
    // Test-only timer. Normal hidden operation has no UI/meter timer.
    let benchmark_timer = Timer::default();
    if let Some(seconds) = benchmark {
        use std::time::Instant;
        let weak = Rc::downgrade(&app);
        let started = Instant::now();
        let mut initial = None;
        let mut maximum_memory = 0;
        let mut samples = Vec::with_capacity((seconds as usize).min(86400));
        let mut not_running = 0;
        let mut maximum_queue = 0;
        let report = benchmark_report.unwrap();
        benchmark_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            let Some(app) = weak.upgrade() else { return; };
            if initial.is_none() { app.hide(); }
            let result = (|| -> Result<bool> {
                let (memory,cpu) = platform::process_resources()?;
                let (first_memory,first_cpu,first_time) = *initial.get_or_insert((memory,cpu,Instant::now()));
                maximum_memory = maximum_memory.max(memory);
                let state = app.engine.status.lock().unwrap().clone();
                if !state.running { not_running += 1; }
                let telemetry = &app.engine.telemetry;
                let queue = telemetry.cable_queue.load(Ordering::Relaxed);
                maximum_queue = maximum_queue.max(queue);
                samples.push(serde_json::json!({"second":started.elapsed().as_secs(),"private_bytes":memory,"queue_frames":queue,"running":state.running}));
                if started.elapsed().as_secs() < seconds { return Ok(false); }
                let duration = first_time.elapsed().as_secs_f64();
                let processors = std::thread::available_parallelism().map_or(1,|count|count.get()) as f64;
                let json = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"measurement":"complete native Slint app, after rendering the overview then hiding to tray; no audio recordings","duration_seconds":duration,"private_bytes_initial":first_memory,"private_bytes_final":memory,"private_bytes_max":maximum_memory,"cpu_total_percent":(cpu-first_cpu)/duration/processors*100.0,"cpu_one_core_percent":(cpu-first_cpu)/duration*100.0,"capture_period_us":telemetry.capture_period_us.load(Ordering::Relaxed),"cable_period_us":telemetry.cable_period_us.load(Ordering::Relaxed),"monitor_period_us":telemetry.monitor_period_us.load(Ordering::Relaxed),"dsp_p99_upper_us":telemetry.dsp_p99_us(),"cable_queue_max_frames":maximum_queue,"cable_underruns":telemetry.cable_underruns.load(Ordering::Relaxed),"monitor_underruns":telemetry.monitor_underruns.load(Ordering::Relaxed),"overrun_frames":telemetry.overruns.load(Ordering::Relaxed),"cable_dropped_frames":telemetry.cable_dropped_frames.load(Ordering::Relaxed),"monitor_dropped_frames":telemetry.monitor_dropped_frames.load(Ordering::Relaxed),"sessions_started":telemetry.sessions_started.load(Ordering::Relaxed),"session_errors":telemetry.session_errors.load(Ordering::Relaxed),"seconds_not_running":not_running,"samples":samples});
                std::fs::write(&report,serde_json::to_vec_pretty(&json)?)?;
                Ok(true)
            })();
            match result {
                Ok(true) => { let _ = slint::quit_event_loop(); },
                Ok(false) => {},
                Err(error) => { eprintln!("Benchmark failed: {error:#}"); std::process::exit(1); }
            }
        });
    }
    if let Some(path) = screenshot {
        if let Some(index) = args
            .iter()
            .position(|arg| arg == "--page")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<i32>().ok())
        {
            ui.set_page(index);
        }
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(700), move || {
            let result = (|| -> Result<()> {
                let ui = weak.upgrade().context("Preview window closed")?;
                let pixels = ui.window().take_snapshot()?;
                let file = std::fs::File::create(path)?;
                let mut encoder = png::Encoder::new(file, pixels.width(), pixels.height());
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()?
                    .write_image_data(pixels.as_bytes())?;
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("Preview failed: {error:#}");
                std::process::exit(1);
            }
            let _ = slint::quit_event_loop();
        });
    }
    slint::run_event_loop()?;
    app.persist();
    app.meters.stop();
    app.save_timer.stop();
    benchmark_timer.stop();
    drop(tray);
    drop(ui);
    drop(app);
    drop(instance);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        let headless = std::env::args().any(|arg| {
            arg == "--self-test"
                || arg == "--ui-self-test"
                || arg == "--version"
                || arg == "--help"
                || arg == "--screenshot"
                || arg == "--benchmark-seconds"
        });
        if !headless {
            platform::error_dialog(&format!("{error:#}"));
        }
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
