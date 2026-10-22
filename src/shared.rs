use crate::config::{EqBand, VoiceSettings};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const COUNT: usize = 25;
pub struct Parameters {
    values: [AtomicU32; COUNT],
    sequence: AtomicU64,
    pub muted: AtomicBool,
    pub monitor: AtomicBool,
    pub monitor_volume: AtomicU32,
}
impl Default for Parameters {
    fn default() -> Self {
        let result = Self {
            values: std::array::from_fn(|_| AtomicU32::new(0)),
            sequence: AtomicU64::new(0),
            muted: AtomicBool::new(false),
            monitor: AtomicBool::new(false),
            monitor_volume: AtomicU32::new(0.5_f32.to_bits()),
        };
        result.store(&VoiceSettings::default());
        result
    }
}
impl Parameters {
    // Single writer: the application/control thread. Readers never wait or retry.
    pub fn store(&self, voice: &VoiceSettings) {
        let mut voice = *voice;
        voice.sanitize();
        let mut values = [0.0; COUNT];
        values[0] = voice.boost;
        values[1] = u8::from(voice.eq_enabled) as f32;
        for (i, band) in voice.bands.iter().enumerate() {
            values[2 + i * 3..5 + i * 3].copy_from_slice(&[band.frequency, band.gain_db, band.q]);
        }
        values[17..25].copy_from_slice(&[
            u8::from(voice.high_pass) as f32,
            u8::from(voice.gate) as f32,
            voice.gate_threshold_db,
            u8::from(voice.compressor) as f32,
            voice.compressor_threshold_db,
            voice.compressor_ratio,
            u8::from(voice.limiter) as f32,
            0.0,
        ]);
        self.sequence.fetch_add(1, Ordering::SeqCst);
        for (destination, value) in self.values.iter().zip(values) {
            destination.store(value.to_bits(), Ordering::SeqCst);
        }
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    pub fn load_changed(&self, last_sequence: &mut u64) -> Option<VoiceSettings> {
        let before = self.sequence.load(Ordering::SeqCst);
        if before == *last_sequence || before & 1 != 0 {
            return None;
        }
        let values: [f32; COUNT] =
            std::array::from_fn(|i| f32::from_bits(self.values[i].load(Ordering::SeqCst)));
        if before != self.sequence.load(Ordering::SeqCst) {
            return None;
        }
        *last_sequence = before;
        Some(VoiceSettings {
            boost: values[0],
            eq_enabled: values[1] > 0.5,
            bands: std::array::from_fn(|i| EqBand {
                frequency: values[2 + i * 3],
                gain_db: values[3 + i * 3],
                q: values[4 + i * 3],
            }),
            high_pass: values[17] > 0.5,
            gate: values[18] > 0.5,
            gate_threshold_db: values[19],
            compressor: values[20] > 0.5,
            compressor_threshold_db: values[21],
            compressor_ratio: values[22],
            limiter: values[23] > 0.5,
        })
    }
}

#[derive(Default)]
pub struct Telemetry {
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub cable_underruns: AtomicU64,
    pub monitor_underruns: AtomicU64,
    pub overruns: AtomicU64,
    pub discontinuities: AtomicU64,
    pub cable_queue: AtomicU32,
    pub monitor_queue: AtomicU32,
    pub capture_period_us: AtomicU32,
    pub cable_period_us: AtomicU32,
    pub monitor_period_us: AtomicU32,
    pub dsp_histogram: [AtomicU64; 16],
    pub dsp_max_ns: AtomicU64,
    pub frames: AtomicU64,
    pub sessions_started: AtomicU64,
    pub session_errors: AtomicU64,
    pub cable_dropped_frames: AtomicU64,
    pub monitor_dropped_frames: AtomicU64,
}
impl Telemetry {
    pub fn reset(&self) {
        self.input_peak.store(0, Ordering::Relaxed);
        self.output_peak.store(0, Ordering::Relaxed);
        self.cable_underruns.store(0, Ordering::Relaxed);
        self.monitor_underruns.store(0, Ordering::Relaxed);
        self.overruns.store(0, Ordering::Relaxed);
        self.discontinuities.store(0, Ordering::Relaxed);
        self.cable_queue.store(0, Ordering::Relaxed);
        self.monitor_queue.store(0, Ordering::Relaxed);
        self.dsp_max_ns.store(0, Ordering::Relaxed);
        self.frames.store(0, Ordering::Relaxed);
        self.sessions_started.store(0, Ordering::Relaxed);
        self.session_errors.store(0, Ordering::Relaxed);
        self.cable_dropped_frames.store(0, Ordering::Relaxed);
        self.monitor_dropped_frames.store(0, Ordering::Relaxed);
        for bucket in &self.dsp_histogram {
            bucket.store(0, Ordering::Relaxed);
        }
    }
    pub fn record_dsp(&self, nanos: u64, frames: u32) {
        let micros = (nanos / 1000).max(1);
        let bucket = (64 - micros.leading_zeros()) as usize;
        self.dsp_histogram[bucket.min(15)].fetch_add(1, Ordering::Relaxed);
        self.dsp_max_ns.fetch_max(nanos, Ordering::Relaxed);
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
    }
    pub fn dsp_p99_us(&self) -> u64 {
        let buckets = self
            .dsp_histogram
            .each_ref()
            .map(|x| x.load(Ordering::Relaxed));
        let total: u64 = buckets.iter().sum();
        if total == 0 {
            return 0;
        }
        let mut sum = 0;
        for (i, count) in buckets.iter().enumerate() {
            sum += count;
            if sum as f64 >= total as f64 * 0.99 {
                return 1 << i;
            }
        }
        1 << 15
    }
}
