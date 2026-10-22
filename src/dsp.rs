use crate::config::VoiceSettings;
use std::f32::consts::PI;

#[derive(Clone, Copy)]
struct Biquad {
    coefficients: [f32; 5],
    target: [f32; 5],
    state: [f32; 2],
    transition: usize,
}
impl Biquad {
    fn new() -> Self {
        Self {
            coefficients: [1.0, 0.0, 0.0, 0.0, 0.0],
            target: [1.0, 0.0, 0.0, 0.0, 0.0],
            state: [0.0; 2],
            transition: 0,
        }
    }
    fn set(&mut self, values: [f32; 5], frames: usize) {
        self.target = values;
        self.transition = frames;
    }
    fn process(&mut self, x: f32) -> f32 {
        if self.transition > 0 {
            for (value, target) in self.coefficients.iter_mut().zip(self.target) {
                *value += (target - *value) / self.transition as f32;
            }
            self.transition -= 1;
        }
        let [b0, b1, b2, a1, a2] = self.coefficients;
        let y = b0 * x + self.state[0];
        self.state[0] = b1 * x - a1 * y + self.state[1];
        self.state[1] = b2 * x - a2 * y;
        // Flush denormals without changing the floating-point environment of other threads.
        for value in &mut self.state {
            if value.abs() < 1.0e-20 {
                *value = 0.0;
            }
        }
        y
    }
}
fn peaking(rate: f32, frequency: f32, gain_db: f32, q: f32) -> [f32; 5] {
    let omega = 2.0 * PI * frequency.min(rate * 0.45) / rate;
    let alpha = omega.sin() / (2.0 * q);
    let a = 10.0_f32.powf(gain_db / 40.0);
    let denominator = 1.0 + alpha / a;
    [
        (1.0 + alpha * a) / denominator,
        -2.0 * omega.cos() / denominator,
        (1.0 - alpha * a) / denominator,
        -2.0 * omega.cos() / denominator,
        (1.0 - alpha / a) / denominator,
    ]
}
fn high_pass(rate: f32) -> [f32; 5] {
    let omega = 2.0 * PI * 80.0 / rate;
    let cos = omega.cos();
    let alpha = omega.sin() / 2.0_f32.sqrt();
    let a0 = 1.0 + alpha;
    [
        (1.0 + cos) / (2.0 * a0),
        -(1.0 + cos) / a0,
        (1.0 + cos) / (2.0 * a0),
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

/// Frequency response of the configured EQ/high-pass, for the static UI curve.
pub fn response_db(settings: &VoiceSettings, rate: f32, frequency: f32) -> f32 {
    let omega = 2.0 * PI * frequency / rate;
    let magnitude = |c: [f32; 5]| {
        let numerator = (c[0] + c[1] * omega.cos() + c[2] * (2.0 * omega).cos()).powi(2)
            + (c[1] * omega.sin() + c[2] * (2.0 * omega).sin()).powi(2);
        let denominator = (1.0 + c[3] * omega.cos() + c[4] * (2.0 * omega).cos()).powi(2)
            + (c[3] * omega.sin() + c[4] * (2.0 * omega).sin()).powi(2);
        10.0 * (numerator / denominator.max(1.0e-20)).max(1.0e-20).log10()
    };
    let mut db = if settings.high_pass {
        magnitude(high_pass(rate))
    } else {
        0.0
    };
    if settings.eq_enabled {
        for band in settings.bands {
            db += magnitude(peaking(rate, band.frequency, band.gain_db, band.q));
        }
    }
    db
}

pub struct Processor {
    rate: f32,
    settings: VoiceSettings,
    filters: [Biquad; 6],
    gain: f32,
    target_gain: f32,
    gate_gain: f32,
    envelope: f32,
    gate_threshold: f32,
    compressor_threshold: f32,
    gain_smoothing: f32,
    attack: f32,
    release: f32,
    gate_release: f32,
    gate_hold: usize,
    compressor_gain: f32,
    compressor_counter: usize,
    compressor_target: f32,
    muted: bool,
}
impl Processor {
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            settings: VoiceSettings::default(),
            filters: [Biquad::new(); 6],
            gain: 1.0,
            target_gain: 1.0,
            gate_gain: 1.0,
            envelope: 0.0,
            gate_threshold: 0.004,
            compressor_threshold: 0.126,
            gain_smoothing: 1.0 - (-1.0 / (0.01 * rate)).exp(),
            attack: 1.0 - (-1.0 / (0.002 * rate)).exp(),
            release: 1.0 - (-1.0 / (0.1 * rate)).exp(),
            gate_release: 1.0 - (-1.0 / (0.06 * rate)).exp(),
            gate_hold: 0,
            compressor_gain: 1.0,
            compressor_counter: 0,
            compressor_target: 1.0,
            muted: false,
        }
    }
    pub fn configure(&mut self, settings: &VoiceSettings) {
        let mut settings = *settings;
        settings.sanitize();
        self.target_gain = settings.boost / 100.0;
        self.gate_threshold = 10.0_f32.powf(settings.gate_threshold_db / 20.0);
        self.compressor_threshold = 10.0_f32.powf(settings.compressor_threshold_db / 20.0);
        let transition = (self.rate * 0.01) as usize;
        self.filters[0].set(
            if settings.high_pass {
                high_pass(self.rate)
            } else {
                [1.0, 0.0, 0.0, 0.0, 0.0]
            },
            transition,
        );
        for (filter, band) in self.filters[1..].iter_mut().zip(settings.bands) {
            filter.set(
                peaking(
                    self.rate,
                    band.frequency,
                    if settings.eq_enabled {
                        band.gain_db
                    } else {
                        0.0
                    },
                    band.q,
                ),
                transition,
            );
        }
        self.settings = settings;
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
    pub fn process(&mut self, input: f32) -> f32 {
        let input = if input.is_finite() && input.abs() >= 1.0e-20 {
            input
        } else {
            0.0
        };
        let level = input.abs();
        let coefficient = if level > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope += coefficient * (level - self.envelope);
        if self.envelope < 1.0e-20 {
            self.envelope = 0.0;
        }
        if self.envelope >= self.gate_threshold {
            self.gate_hold = (0.05 * self.rate) as usize;
        }
        self.gate_hold = self.gate_hold.saturating_sub(1);
        let gate_target = if !self.settings.gate || self.gate_hold > 0 {
            1.0
        } else {
            0.0
        };
        let gate_coefficient = if gate_target > self.gate_gain {
            self.attack
        } else {
            self.gate_release
        };
        self.gate_gain += gate_coefficient * (gate_target - self.gate_gain);
        if self.gate_gain < 1.0e-20 {
            self.gate_gain = 0.0;
        }
        let target = if self.muted { 0.0 } else { self.target_gain };
        self.gain += self.gain_smoothing * (target - self.gain);
        if self.gain < 1.0e-20 {
            self.gain = 0.0;
        }
        let mut output = input * self.gate_gain;
        for filter in &mut self.filters {
            output = filter.process(output);
        }
        output *= self.gain;
        // Evaluate the compressor curve at 1 kHz, rather than powf per sample.
        self.compressor_counter += 1;
        if self.compressor_counter >= (self.rate / 1000.0).max(1.0) as usize {
            self.compressor_counter = 0;
            let level = self.envelope * self.gain;
            self.compressor_target = if self.settings.compressor
                && level > self.compressor_threshold
            {
                (level / self.compressor_threshold).powf(1.0 / self.settings.compressor_ratio - 1.0)
            } else {
                1.0
            };
        }
        let coefficient = if self.compressor_target < self.compressor_gain {
            self.attack
        } else {
            self.release
        };
        self.compressor_gain += coefficient * (self.compressor_target - self.compressor_gain);
        output *= self.compressor_gain;
        if self.settings.limiter {
            // Zero-lookahead soft peak limiter. Unity below -3 dBFS; bounded above it.
            let magnitude = output.abs();
            if magnitude > 0.7 {
                let excess = magnitude - 0.7;
                output = output.signum() * (0.7 + 0.28 * excess / (0.28 + excess));
            }
        }
        if output.is_finite() {
            output.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}
