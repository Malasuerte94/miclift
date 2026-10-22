//! WASAPI shared-mode engine. COM objects stay on their owning audio threads.
//! Allocation, device discovery and recovery happen outside the processing loops.
use crate::{
    config::Config,
    dsp::Processor,
    resample::Resampler,
    shared::{Parameters, Telemetry},
};
use anyhow::{Context, Result, anyhow, bail};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize, STGM_READ,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            },
            Threading::{
                AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
                WaitForSingleObject,
            },
        },
    },
    core::{HSTRING, Interface, PWSTR, w},
};

struct Com(bool);
impl Com {
    fn new() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            Ok(Self(true))
        } else if hr.0 == -2147417850 {
            Ok(Self(false))
        } else {
            Err(windows::core::Error::from_hresult(hr).into())
        }
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Mmcss(Option<HANDLE>);
impl Mmcss {
    fn new() -> Self {
        let mut index = 0;
        Self(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index).ok() })
    }
}
impl Drop for Mmcss {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(handle);
            }
        }
    }
}
fn take_string(pointer: PWSTR) -> String {
    let value = unsafe { pointer.to_string().unwrap_or_default() };
    unsafe {
        CoTaskMemFree(Some(pointer.0.cast()));
    }
    value
}
fn enumerator() -> Result<IMMDeviceEnumerator> {
    Ok(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub is_cable: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Devices {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}
pub fn devices() -> Result<Devices> {
    let _com = Com::new()?;
    let enumerator = enumerator()?;
    let list = |flow| -> Result<Vec<DeviceInfo>> {
        let default = unsafe { enumerator.GetDefaultAudioEndpoint(flow, eCommunications) }
            .ok()
            .and_then(|d| unsafe { d.GetId() }.ok())
            .map(take_string)
            .unwrap_or_default();
        let collection = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
        let mut result = Vec::new();
        for index in 0..unsafe { collection.GetCount()? } {
            let device = unsafe { collection.Item(index)? };
            let id = take_string(unsafe { device.GetId()? });
            let properties = unsafe { device.OpenPropertyStore(STGM_READ)? };
            let mut value = unsafe { properties.GetValue(&PKEY_Device_FriendlyName)? };
            let name = unsafe { PropVariantToStringAlloc(&value) }
                .map(take_string)
                .unwrap_or_else(|_| id.clone());
            unsafe {
                let _ = PropVariantClear(&mut value);
            }
            let lower = name.to_lowercase();
            let is_cable = lower.contains("vb-audio") && lower.contains("cable");
            result.push(DeviceInfo {
                is_default: id == default,
                id,
                name,
                is_cable,
            });
        }
        Ok(result)
    };
    Ok(Devices {
        inputs: list(eCapture)?,
        outputs: list(eRender)?,
    })
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub float: bool,
    pub block: usize,
}
impl Format {
    fn parse(pointer: *const WAVEFORMATEX) -> Result<Self> {
        let format = unsafe { pointer.read_unaligned() };
        let tag = format.wFormatTag;
        let subtype = if tag == 0xfffe {
            anyhow::ensure!(format.cbSize >= 22, "Invalid extensible mix format");
            unsafe {
                (pointer as *const WAVEFORMATEXTENSIBLE)
                    .read_unaligned()
                    .SubFormat
                    .data1 as u16
            }
        } else {
            tag
        };
        let result = Self {
            rate: format.nSamplesPerSec,
            channels: format.nChannels,
            bits: format.wBitsPerSample,
            float: subtype == 3,
            block: format.nBlockAlign as usize,
        };
        anyhow::ensure!(
            result.rate > 0 && result.channels > 0 && result.channels <= 32,
            "Invalid audio format"
        );
        anyhow::ensure!(
            (subtype == 3 && result.bits == 32)
                || (subtype == 1 && [8, 16, 24, 32].contains(&result.bits)),
            "Unsupported Windows mix format: tag={subtype}, bits={}",
            result.bits
        );
        anyhow::ensure!(
            result.block == result.channels as usize * result.bits as usize / 8,
            "Invalid audio frame alignment"
        );
        Ok(result)
    }
    // SAFETY: callers provide a complete WASAPI frame, or flag the buffer silent.
    unsafe fn read(&self, frame: *const u8) -> f32 {
        let bytes = self.bits as usize / 8;
        let mut sum = 0.0;
        for channel in 0..self.channels as usize {
            let pointer = unsafe { frame.add(channel * bytes) };
            let sample = unsafe {
                match (self.float, self.bits) {
                    (true, 32) => pointer.cast::<f32>().read_unaligned(),
                    (_, 8) => (pointer.read() as f32 - 128.0) / 128.0,
                    (_, 16) => pointer.cast::<i16>().read_unaligned() as f32 / 32768.0,
                    (_, 24) => {
                        let raw = (pointer.read() as i32)
                            | ((pointer.add(1).read() as i32) << 8)
                            | ((pointer.add(2).read() as i32) << 16);
                        ((raw << 8) >> 8) as f32 / 8388608.0
                    }
                    _ => pointer.cast::<i32>().read_unaligned() as f32 / 2147483648.0,
                }
            };
            if sample.is_finite() {
                sum += sample;
            }
        }
        sum / self.channels as f32
    }
    unsafe fn write(&self, frame: *mut u8, sample: f32) {
        let sample = sample.clamp(-1.0, 1.0);
        for channel in 0..self.channels as usize {
            let pointer = unsafe { frame.add(channel * self.bits as usize / 8) };
            unsafe {
                match (self.float, self.bits) {
                    (true, 32) => pointer.cast::<f32>().write_unaligned(sample),
                    (_, 8) => pointer.write((sample * 127.0 + 128.0) as u8),
                    (_, 16) => pointer
                        .cast::<i16>()
                        .write_unaligned((sample * 32767.0) as i16),
                    (_, 24) => {
                        let raw = (sample * 8388607.0) as i32;
                        pointer.write(raw as u8);
                        pointer.add(1).write((raw >> 8) as u8);
                        pointer.add(2).write((raw >> 16) as u8);
                    }
                    _ => pointer
                        .cast::<i32>()
                        .write_unaligned((sample as f64 * 2147483647.0) as i32),
                }
            }
        }
    }
}
struct MixFormat(*mut WAVEFORMATEX);
impl Drop for MixFormat {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}
pub(crate) struct Session {
    client: IAudioClient,
    event: OwnedHandle,
    pub format: Format,
    pub period: u32,
    pub buffer_frames: u32,
    _com: Com,
}
impl Session {
    pub(crate) fn open(id: &str, capture: bool, loopback: bool) -> Result<Self> {
        let com = Com::new()?;
        let enumerator = enumerator()?;
        let device = if id.is_empty() {
            unsafe {
                enumerator.GetDefaultAudioEndpoint(
                    if capture && !loopback {
                        eCapture
                    } else {
                        eRender
                    },
                    eCommunications,
                )?
            }
        } else {
            unsafe { enumerator.GetDevice(&HSTRING::from(id))? }
        };
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let format_pointer = MixFormat(unsafe { client.GetMixFormat()? });
        let format = Format::parse(format_pointer.0)?;
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | if loopback {
                AUDCLNT_STREAMFLAGS_LOOPBACK
            } else {
                0
            };
        let mut period = 0;
        let low_latency = if !loopback {
            if let Ok(client3) = client.cast::<IAudioClient3>() {
                let (mut default, mut fundamental, mut min, mut max) = (0, 0, 0, 0);
                if unsafe {
                    client3.GetSharedModeEnginePeriod(
                        format_pointer.0,
                        &mut default,
                        &mut fundamental,
                        &mut min,
                        &mut max,
                    )
                }
                .is_ok()
                {
                    let initialized = unsafe {
                        client3.InitializeSharedAudioStream(flags, min, format_pointer.0, None)
                    }
                    .is_ok();
                    if initialized {
                        period = min;
                    }
                    initialized
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if !low_latency {
            // A locked shared engine may reject IAudioClient3's minimum. Fresh client fallback.
            let fallback: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
            let mut duration = 0;
            unsafe {
                fallback.GetDevicePeriod(Some(&mut duration), None)?;
            }
            period = ((duration as u64 * format.rate as u64) / 10_000_000) as u32;
            unsafe {
                fallback.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    flags,
                    duration * 2,
                    0,
                    format_pointer.0,
                    None,
                )?;
            }
            return Self::finish(fallback, format, period, com);
        }
        Self::finish(client, format, period, com)
    }
    fn finish(client: IAudioClient, format: Format, period: u32, com: Com) -> Result<Self> {
        let event = OwnedHandle(unsafe { CreateEventW(None, false, false, None)? });
        unsafe {
            client.SetEventHandle(event.0)?;
        }
        let buffer_frames = unsafe { client.GetBufferSize()? };
        Ok(Self {
            client,
            event,
            format,
            period,
            buffer_frames,
            _com: com,
        })
    }
    fn wait(&self, stop: &AtomicBool) -> Result<bool> {
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            match unsafe { WaitForSingleObject(self.event.0, 50) } {
                WAIT_OBJECT_0 => return Ok(true),
                WAIT_TIMEOUT => {
                    // Calling into WASAPI makes device invalidation observable even while suspended.
                    unsafe {
                        self.client.GetCurrentPadding()?;
                    }
                }
                _ => bail!("WASAPI event wait failed"),
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub running: bool,
    pub requested: bool,
    pub message: String,
    pub devices: Devices,
    pub input_rate: u32,
}
enum Command {
    Start(Config),
    Stop,
    Quit,
}
pub struct Engine {
    sender: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
    pub parameters: Arc<Parameters>,
    pub telemetry: Arc<Telemetry>,
    pub status: Arc<Mutex<Status>>,
}
struct Pipeline {
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    errors: mpsc::Receiver<(bool, String)>,
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
impl Engine {
    pub fn new() -> Self {
        let parameters = Arc::new(Parameters::default());
        let telemetry = Arc::new(Telemetry::default());
        let status = Arc::new(Mutex::new(Status {
            running: false,
            requested: false,
            message: "Ready — choose your microphone".into(),
            devices: devices().unwrap_or_default(),
            input_rate: 0,
        }));
        let (sender, receiver) = mpsc::channel();
        let (p, t, s) = (parameters.clone(), telemetry.clone(), status.clone());
        let worker = thread::spawn(move || {
            let mut config: Option<Config> = None;
            let mut pipeline: Option<Pipeline> = None;
            let mut retry = Instant::now();
            let mut discovery = Instant::now();
            let mut monitor_failed = false;
            loop {
                match receiver.recv_timeout(Duration::from_millis(250)) {
                    Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Command::Stop) => {
                        pipeline = None;
                        config = None;
                        let mut state = s.lock().unwrap();
                        state.running = false;
                        state.requested = false;
                        state.message = "Stopped".into();
                        t.input_peak.store(0, Ordering::Relaxed);
                        t.output_peak.store(0, Ordering::Relaxed);
                    }
                    Ok(Command::Start(next)) => {
                        pipeline = None;
                        t.reset();
                        monitor_failed = false;
                        config = Some(next);
                        retry = Instant::now();
                        s.lock().unwrap().requested = true;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if discovery.elapsed() >= Duration::from_secs(2) {
                    if let Ok(devices) = devices() {
                        s.lock().unwrap().devices = devices;
                    }
                    discovery = Instant::now();
                }
                if let Some(active) = &pipeline
                    && let Ok((monitor_failure, error)) = active.errors.try_recv()
                {
                    t.session_errors.fetch_add(1, Ordering::Relaxed);
                    pipeline = None;
                    let mut state = s.lock().unwrap();
                    state.running = false;
                    if monitor_failure {
                        monitor_failed = true;
                        p.monitor.store(false, Ordering::Relaxed);
                        state.message =
                            format!("Live Listen disconnected; restoring microphone: {error}");
                        retry = Instant::now();
                    } else {
                        state.message = format!("Reconnecting: {error}");
                        retry = Instant::now() + Duration::from_secs(2);
                    }
                }
                if pipeline.is_none()
                    && Instant::now() >= retry
                    && let Some(config) = &config
                {
                    match Pipeline::start(config, p.clone(), t.clone()) {
                        Ok((active, rate, monitor_unavailable)) => {
                            t.sessions_started.fetch_add(1, Ordering::Relaxed);
                            pipeline = Some(active);
                            let mut state = s.lock().unwrap();
                            state.running = true;
                            state.input_rate = rate;
                            state.message = if monitor_failed || monitor_unavailable {
                                "Live · microphone → VB-CABLE · Live Listen output unavailable; select an output and re-enable it".into()
                            } else {
                                "Live · processed microphone → VB-CABLE".into()
                            };
                        }
                        Err(error) => {
                            t.session_errors.fetch_add(1, Ordering::Relaxed);
                            let mut state = s.lock().unwrap();
                            state.running = false;
                            state.message = format!("Waiting for audio devices: {error:#}");
                            retry = Instant::now() + Duration::from_secs(3);
                        }
                    }
                }
            }
            drop(pipeline);
        });
        Self {
            sender,
            worker: Some(worker),
            parameters,
            telemetry,
            status,
        }
    }
    pub fn start(&self, config: &Config) {
        let _ = self.sender.send(Command::Start(config.clone()));
    }
    pub fn stop(&self) {
        let _ = self.sender.send(Command::Stop);
    }
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type CaptureSinks = (Producer<f32>, Option<Producer<f32>>);
impl Pipeline {
    fn start(
        config: &Config,
        parameters: Arc<Parameters>,
        telemetry: Arc<Telemetry>,
    ) -> Result<(Self, u32, bool)> {
        let devices = devices()?;
        let cable_id = if config.cable_id.is_empty() {
            devices
                .outputs
                .iter()
                .find(|d| d.is_cable)
                .context("VB-CABLE is not installed. Open Settings → VB-CABLE setup")?
                .id
                .clone()
        } else {
            config.cable_id.clone()
        };
        anyhow::ensure!(
            devices
                .outputs
                .iter()
                .any(|d| d.id == cable_id && d.is_cable),
            "Selected virtual cable is unavailable"
        );
        let input_id = if config.input_id.is_empty() {
            devices
                .inputs
                .iter()
                .find(|d| d.is_default && !d.is_cable)
                .or_else(|| devices.inputs.iter().find(|d| !d.is_cable))
                .context("No microphone available")?
                .id
                .clone()
        } else {
            config.input_id.clone()
        };
        anyhow::ensure!(
            !devices
                .inputs
                .iter()
                .any(|d| d.id == input_id && d.is_cable),
            "The virtual cable cannot be used as its own microphone"
        );
        let monitor_requested = parameters.monitor.load(Ordering::Relaxed);
        let monitor_id = if config.monitor_id.is_empty() {
            devices
                .outputs
                .iter()
                .find(|d| d.is_default && !d.is_cable)
                .or_else(|| devices.outputs.iter().find(|d| !d.is_cable))
                .map(|d| d.id.clone())
                .unwrap_or_default()
        } else {
            config.monitor_id.clone()
        };
        let monitor_valid = !monitor_id.is_empty()
            && monitor_id != cable_id
            && devices
                .outputs
                .iter()
                .any(|d| d.id == monitor_id && !d.is_cable);
        let monitoring = monitor_requested && monitor_valid;
        if monitor_requested && !monitor_valid {
            parameters.monitor.store(false, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (errors_tx, errors) = mpsc::channel();
        let mut pipeline = Self {
            stop: stop.clone(),
            workers: Vec::new(),
            errors,
        };
        let (ready_tx, ready_rx) = mpsc::channel();
        let (sinks_tx, sinks_rx) = mpsc::channel::<CaptureSinks>();
        let (p, t, stopping, errors) = (
            parameters.clone(),
            telemetry.clone(),
            stop.clone(),
            errors_tx.clone(),
        );
        pipeline.workers.push(thread::spawn(move || {
            let result = (|| -> Result<()> {
                let session = Session::open(&input_id, true, false)?;
                ready_tx.send((session.format.rate, session.period)).ok();
                let (cable, monitor) = sinks_rx.recv().context("Audio startup was cancelled")?;
                capture(session, cable, monitor, &stopping, &p, &t)
            })();
            if let Err(error) = result {
                let message = format!("Capture: {error:#}");
                let _ = errors.send((false, message));
            }
        }));
        let (rate, period) = ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| {
            anyhow!("Unable to open microphone; check Windows microphone privacy permissions")
        })?;
        let capacity = ((rate as usize / 4).max(4096)).next_power_of_two();
        let (cable_tx, cable_rx) = RingBuffer::new(capacity);
        let (monitor_tx, monitor_rx) = if monitoring {
            let (tx, rx) = RingBuffer::new(capacity);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        for (id, consumer, monitor) in std::iter::once((cable_id, cable_rx, false))
            .chain(monitor_rx.map(|rx| (monitor_id, rx, true)))
        {
            let (p, t, stopping, errors) = (
                parameters.clone(),
                telemetry.clone(),
                stop.clone(),
                errors_tx.clone(),
            );
            let (ready_tx, ready_rx) = mpsc::channel();
            pipeline.workers.push(thread::spawn(move || {
                let result = (|| -> Result<()> {
                    let session = Session::open(&id, false, false)?;
                    let target = (period as f64)
                        .max(session.period as f64 * rate as f64 / session.format.rate as f64)
                        .ceil() as usize
                        + 32;
                    let resampler = Resampler::new(rate, session.format.rate, target);
                    ready_tx.send(()).ok();
                    render(session, consumer, resampler, monitor, &stopping, &p, &t)
                })();
                if let Err(error) = result {
                    let _ = errors.send((
                        monitor,
                        format!(
                            "{}: {error:#}",
                            if monitor { "Live Listen" } else { "VB-CABLE" }
                        ),
                    ));
                }
            }));
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .context("Unable to open audio output")?;
        }
        sinks_tx
            .send((cable_tx, monitor_tx))
            .map_err(|_| anyhow!("Microphone startup failed"))?;
        Ok((pipeline, rate, monitor_requested && !monitor_valid))
    }
}

fn capture(
    session: Session,
    mut cable: Producer<f32>,
    mut monitor: Option<Producer<f32>>,
    stop: &AtomicBool,
    parameters: &Parameters,
    telemetry: &Telemetry,
) -> Result<()> {
    let service: IAudioCaptureClient = unsafe { session.client.GetService()? };
    let mut processor = Processor::new(session.format.rate as f32);
    let mut sequence = 0;
    let _priority = Mmcss::new();
    telemetry.capture_period_us.store(
        (session.period as u64 * 1_000_000 / session.format.rate as u64) as u32,
        Ordering::Relaxed,
    );
    unsafe {
        session.client.Start()?;
    }
    while session.wait(stop)? {
        while unsafe { service.GetNextPacketSize()? } > 0 {
            let (mut pointer, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
            unsafe {
                service.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None)?;
            }
            let start = Instant::now();
            if let Some(settings) = parameters.load_changed(&mut sequence) {
                processor.configure(&settings);
            }
            processor.set_muted(parameters.muted.load(Ordering::Relaxed));
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                telemetry.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
            let (mut input_peak, mut output_peak) = (0.0_f32, 0.0_f32);
            for frame in 0..frames as usize {
                let input = if silent {
                    0.0
                } else {
                    unsafe {
                        session
                            .format
                            .read(pointer.add(frame * session.format.block))
                    }
                };
                let output = processor.process(input);
                input_peak = input_peak.max(input.abs());
                output_peak = output_peak.max(output.abs());
                if cable.push(output).is_err() {
                    telemetry.overruns.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(sink) = &mut monitor
                    && sink.push(output).is_err()
                {
                    telemetry.overruns.fetch_add(1, Ordering::Relaxed);
                }
            }
            telemetry
                .input_peak
                .fetch_max(input_peak.to_bits(), Ordering::Relaxed);
            telemetry
                .output_peak
                .fetch_max(output_peak.to_bits(), Ordering::Relaxed);
            telemetry.record_dsp(start.elapsed().as_nanos() as u64, frames);
            unsafe {
                service.ReleaseBuffer(frames)?;
            }
        }
    }
    Ok(())
}
fn render(
    session: Session,
    mut input: Consumer<f32>,
    mut resampler: Resampler,
    monitor: bool,
    stop: &AtomicBool,
    parameters: &Parameters,
    telemetry: &Telemetry,
) -> Result<()> {
    let service: IAudioRenderClient = unsafe { session.client.GetService()? };
    let period = (session.period as u64 * 1_000_000 / session.format.rate as u64) as u32;
    if monitor {
        telemetry.monitor_period_us.store(period, Ordering::Relaxed);
    } else {
        telemetry.cable_period_us.store(period, Ordering::Relaxed);
    }
    let mut gain = if monitor { 0.0 } else { 1.0 };
    let smoothing = 1.0 - (-1.0 / (session.format.rate as f32 * 0.01)).exp();
    let _priority = Mmcss::new();
    unsafe {
        let prime = session.period.min(session.buffer_frames);
        service.GetBuffer(prime)?;
        service.ReleaseBuffer(prime, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        session.client.Start()?;
    }
    while session.wait(stop)? {
        // Buffer capacity is not a desired queue depth. Keep only one negotiated
        // period ahead instead of filling every available frame in the endpoint.
        let padding = unsafe { session.client.GetCurrentPadding()? };
        let available = session
            .period
            .min(session.buffer_frames)
            .saturating_sub(padding);
        if available == 0 {
            continue;
        }
        let ready = resampler.begin_block(&mut input, available as usize);
        let pointer = unsafe { service.GetBuffer(available)? };
        let target = if monitor {
            if parameters.monitor.load(Ordering::Relaxed) {
                f32::from_bits(parameters.monitor_volume.load(Ordering::Relaxed)).clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            1.0
        };
        if ready {
            for frame in 0..available as usize {
                gain += smoothing * (target - gain);
                let sample = resampler.next(&mut input) * gain;
                unsafe {
                    session
                        .format
                        .write(pointer.add(frame * session.format.block), sample);
                }
            }
        }
        unsafe {
            service.ReleaseBuffer(
                available,
                if ready {
                    0
                } else {
                    AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
                },
            )?;
        }
        let (queue, underruns) = if monitor {
            (&telemetry.monitor_queue, &telemetry.monitor_underruns)
        } else {
            (&telemetry.cable_queue, &telemetry.cable_underruns)
        };
        queue.store(input.slots() as u32, Ordering::Relaxed);
        underruns.store(resampler.underruns, Ordering::Relaxed);
        let dropped = if monitor {
            &telemetry.monitor_dropped_frames
        } else {
            &telemetry.cable_dropped_frames
        };
        dropped.store(resampler.dropped, Ordering::Relaxed);
    }
    Ok(())
}

/// Software cable round trip only. Uses a test tone and WASAPI capture; no mic or recording files.
pub fn cable_latency(cable_id: &str, seconds: u64) -> Result<serde_json::Value> {
    let listing = devices()?;
    let output = listing
        .outputs
        .iter()
        .find(|d| d.is_cable && (cable_id.is_empty() || d.id == cable_id))
        .context("No VB-CABLE render endpoint")?;
    let input = listing
        .inputs
        .iter()
        .find(|d| d.is_cable)
        .context("No VB-CABLE capture endpoint")?;
    let render = Session::open(&output.id, false, false)?;
    let capture = Session::open(&input.id, true, false)?;
    let rate = render.format.rate;
    let capture_rate = capture.format.rate;
    let render_client: IAudioRenderClient = unsafe { render.client.GetService()? };
    let capture_client: IAudioCaptureClient = unsafe { capture.client.GetService()? };
    let clock: IAudioClock = unsafe { render.client.GetService()? };
    let frequency = unsafe { clock.GetFrequency()? };
    // The same continuous chirp is sampled at each endpoint's native rate.
    let signal = |time: f64| -> f32 {
        if !(0.0..0.08).contains(&time) {
            return 0.0;
        }
        let phase = std::f64::consts::TAU * (300.0 * time + 0.5 * 50000.0 * time * time);
        (phase.sin() * (std::f64::consts::PI * time / 0.08).sin() * 0.05) as f32
    };
    let reference: Vec<f32> = (0..(capture_rate as f64 * 0.08) as usize)
        .map(|i| signal(i as f64 / capture_rate as f64))
        .collect();
    let mut recorded = Vec::with_capacity(capture_rate as usize * (seconds as usize + 2));
    let mut stamps = Vec::with_capacity(seconds as usize * 1000);
    let marker = rate as usize / 2;
    let mut submitted = 0;
    let mut discontinuities = 0;
    unsafe {
        render_client.GetBuffer(render.buffer_frames)?;
        render_client.ReleaseBuffer(render.buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        capture.client.Start()?;
        render.client.Start()?;
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds.clamp(2, 30)) {
        unsafe {
            WaitForSingleObject(render.event.0, 2);
        }
        let available = render
            .buffer_frames
            .saturating_sub(unsafe { render.client.GetCurrentPadding()? });
        if available > 0 {
            let pointer = unsafe { render_client.GetBuffer(available)? };
            for frame in 0..available as usize {
                let index = submitted + frame;
                let sample = index
                    .checked_sub(marker)
                    .map_or(0.0, |i| signal(i as f64 / rate as f64));
                unsafe {
                    render
                        .format
                        .write(pointer.add(frame * render.format.block), sample);
                }
            }
            unsafe {
                render_client.ReleaseBuffer(available, 0)?;
            }
            submitted += available as usize;
        }
        while unsafe { capture_client.GetNextPacketSize()? } > 0 {
            let (mut pointer, mut count, mut flags, mut qpc) = (std::ptr::null_mut(), 0, 0, 0);
            unsafe {
                capture_client.GetBuffer(
                    &mut pointer,
                    &mut count,
                    &mut flags,
                    None,
                    Some(&mut qpc),
                )?;
            }
            stamps.push((recorded.len(), qpc));
            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                discontinuities += 1;
            }
            for frame in 0..count as usize {
                recorded.push(if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    0.0
                } else {
                    unsafe {
                        capture
                            .format
                            .read(pointer.add(frame * capture.format.block))
                    }
                });
            }
            unsafe {
                capture_client.ReleaseBuffer(count)?;
            }
        }
    }
    let (mut position, mut clock_qpc) = (0, 0);
    unsafe {
        clock.GetPosition(&mut position, Some(&mut clock_qpc))?;
    }
    anyhow::ensure!(recorded.len() > reference.len(), "No cable capture samples");
    let reference_energy: f64 = reference.iter().map(|x| (*x as f64).powi(2)).sum();
    let score = |offset: usize, stride: usize| {
        let (mut dot, mut energy) = (0.0, 0.0);
        for index in (0..reference.len()).step_by(stride) {
            let sample = recorded[offset + index] as f64;
            dot += sample * reference[index] as f64;
            energy += sample * sample;
        }
        dot / (energy * reference_energy / stride as f64)
            .sqrt()
            .max(1.0e-20)
    };
    let (mut best, mut correlation) = (0, 0.0);
    for offset in (0..recorded.len() - reference.len()).step_by(4) {
        let value = score(offset, 4);
        if value > correlation {
            best = offset;
            correlation = value;
        }
    }
    let coarse = best;
    correlation = 0.0;
    for offset in coarse.saturating_sub(8)..=(coarse + 8).min(recorded.len() - reference.len() - 1)
    {
        let value = score(offset, 1);
        if value > correlation {
            best = offset;
            correlation = value;
        }
    }
    anyhow::ensure!(
        correlation > 0.85,
        "Probe signal not detected reliably (correlation {correlation:.3}); stop other cable producers and retry"
    );
    let (packet_index, packet_qpc) = stamps
        .iter()
        .rev()
        .find(|(index, _)| *index <= best)
        .copied()
        .context("No capture timestamps")?;
    let captured_qpc =
        packet_qpc as f64 + (best - packet_index) as f64 * 10_000_000.0 / capture_rate as f64;
    let marker_position =
        (render.buffer_frames as usize + marker) as f64 * frequency as f64 / rate as f64;
    let rendered_qpc =
        clock_qpc as f64 + (marker_position - position as f64) * 10_000_000.0 / frequency as f64;
    Ok(
        serde_json::json!({"measurement":"VB-CABLE endpoint-clock offset; excludes microphone, MicLift DSP and physical listening", "latency_ms":(captured_qpc-rendered_qpc)/10000.0,"correlation":correlation,"render_sample_rate":rate,"capture_sample_rate":capture_rate,"discontinuities":discontinuities,"capture_period_frames":capture.period,"render_period_frames":render.period,"note":"Depends on driver-reported clocks. Not acoustic end-to-end latency."}),
    )
}
