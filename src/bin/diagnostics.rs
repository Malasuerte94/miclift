use anyhow::{Context, Result, bail};
use miclift::{audio, config::Config, dsp::Processor, platform, shared::Telemetry};
use serde_json::json;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|s| s == flag)
            .and_then(|i| args.get(i + 1))
    };
    let result = if args.iter().any(|s| s == "--self-test") {
        miclift::self_test()?;
        json!({"self_test":"passed"})
    } else if args.iter().any(|s| s == "--list") {
        serde_json::to_value(audio::devices()?)?
    } else if args.iter().any(|s| s == "--cable-probe") {
        audio::cable_latency(value("--cable").map_or("", String::as_str), 3)?
    } else if args.iter().any(|s| s == "--dsp-benchmark") {
        let mut processor = Processor::new(48000.0);
        let mut config = Config::default();
        config.voice.boost = 500.0;
        config.voice.high_pass = true;
        config.voice.gate = true;
        config.voice.compressor = true;
        config.voice.bands[2].gain_db = 3.0;
        processor.configure(&config.voice);
        let stats = Telemetry::default();
        let mut phase = 0.0_f32;
        let started = Instant::now();
        for _ in 0..10000 {
            let block_start = Instant::now();
            for _ in 0..128 {
                phase += 0.131;
                let _ = std::hint::black_box(processor.process(phase.sin() * 0.1));
            }
            stats.record_dsp(block_start.elapsed().as_nanos() as u64, 128);
        }
        json!({"frames":1280000,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"block_frames":128,"sample_rate":48000,"dsp_p99_upper_us":stats.dsp_p99_us(),"dsp_max_us":stats.dsp_max_ns.load(Ordering::Relaxed) as f64/1000.0,"all_basic_effects":true})
    } else if let Some(seconds) = value("--run-seconds") {
        let seconds: u64 = seconds.parse().context("Invalid duration")?;
        anyhow::ensure!(
            (1..=86400).contains(&seconds),
            "Duration must be 1..86400 seconds"
        );
        let mut config = Config::load()?;
        if let Some(input) = value("--input") {
            config.input_id.clone_from(input);
        }
        if let Some(cable) = value("--cable") {
            config.cable_id.clone_from(cable);
        }
        if let Some(output) = value("--monitor-output") {
            config.monitor_id.clone_from(output);
        }
        let engine = audio::Engine::new();
        engine.parameters.store(&config.voice);
        if args.iter().any(|s| s == "--monitor") {
            engine.parameters.monitor.store(true, Ordering::Relaxed);
        }
        engine
            .parameters
            .monitor_volume
            .store(config.monitor_volume.to_bits(), Ordering::Relaxed);
        engine.start(&config);
        let started = Instant::now();
        let mut ready = None;
        while started.elapsed() < Duration::from_secs(10) {
            let status = engine.status.lock().unwrap().clone();
            if status.running {
                ready = Some(status);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if ready.is_none() {
            bail!("{}", engine.status.lock().unwrap().message);
        }
        let (initial_memory, initial_cpu) = platform::process_resources()?;
        let mut max_memory = initial_memory;
        let mut max_queue = 0;
        let mut failures = 0;
        let run_start = Instant::now();
        let mut samples = Vec::new();
        while run_start.elapsed() < Duration::from_secs(seconds) {
            std::thread::sleep(Duration::from_secs(1));
            let (memory, _) = platform::process_resources()?;
            max_memory = max_memory.max(memory);
            let queue = engine.telemetry.cable_queue.load(Ordering::Relaxed);
            max_queue = max_queue.max(queue);
            if !engine.status.lock().unwrap().running {
                failures += 1;
            }
            if samples.len() < 2880
                && (run_start.elapsed().as_secs() < 10
                    || run_start.elapsed().as_secs().is_multiple_of(5))
            {
                samples.push(json!({"second":run_start.elapsed().as_secs(),"private_bytes":memory,"queue_frames":queue}));
            }
        }
        let (final_memory, final_cpu) = platform::process_resources()?;
        let cpu_seconds = final_cpu - initial_cpu;
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
        let stats = &engine.telemetry;
        let result = json!({"duration_seconds":run_start.elapsed().as_secs_f64(),"private_bytes_initial":initial_memory,"private_bytes_final":final_memory,"private_bytes_max":max_memory,"cpu_total_percent":cpu_seconds/run_start.elapsed().as_secs_f64()/cpus*100.0,"cpu_one_core_percent":cpu_seconds/run_start.elapsed().as_secs_f64()*100.0,"cable_queue_max_frames":max_queue,"cable_underruns":stats.cable_underruns.load(Ordering::Relaxed),"monitor_underruns":stats.monitor_underruns.load(Ordering::Relaxed),"overrun_frames":stats.overruns.load(Ordering::Relaxed),"cable_dropped_frames":stats.cable_dropped_frames.load(Ordering::Relaxed),"monitor_dropped_frames":stats.monitor_dropped_frames.load(Ordering::Relaxed),"sessions_started":stats.sessions_started.load(Ordering::Relaxed),"session_errors":stats.session_errors.load(Ordering::Relaxed),"capture_discontinuities":stats.discontinuities.load(Ordering::Relaxed),"dsp_p99_upper_us":stats.dsp_p99_us(),"capture_period_us":stats.capture_period_us.load(Ordering::Relaxed),"cable_period_us":stats.cable_period_us.load(Ordering::Relaxed),"monitor_period_us":stats.monitor_period_us.load(Ordering::Relaxed),"seconds_not_running":failures,"samples":samples,"measurement":"headless engine process resources; not the Slint window or physical end-to-end latency"});
        engine.stop();
        result
    } else {
        println!(
            "MicLift diagnostics\n  --self-test             DSP check, no devices\n  --list                  Endpoint IDs and names\n  --dsp-benchmark         DSP block timings, no devices\n  --cable-probe           Test signal through VB-CABLE; no microphone\n  --run-seconds N         Live microphone -> cable resource/soak test\n  --input ID --cable ID   Override saved endpoints\n  --monitor [--monitor-output ID]  Enable live listening during soak\n  --report PATH          Save JSON results (never audio)"
        );
        return Ok(());
    };
    let text = serde_json::to_string_pretty(&result)?;
    if let Some(path) = value("--report") {
        std::fs::write(path, &text)?;
    }
    println!("{text}");
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
