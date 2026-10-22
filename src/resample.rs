//! Bounded streaming windowed-sinc ASRC. No allocation or blocking in render().
//! The 32-tap interpolation kernel is precomputed; its group delay is 16 input frames.
use rtrb::Consumer;
use std::f64::consts::PI;
const TAPS: usize = 32;
const PHASES: usize = 256;

pub struct Resampler {
    coefficients: Box<[[f32; TAPS]; PHASES + 1]>,
    history: [f32; TAPS],
    cursor: usize,
    phase: f64,
    ratio: f64,
    correction: f64,
    target: usize,
    primed: bool,
    pub underruns: u64,
    pub dropped: u64,
}
impl Resampler {
    pub fn new(input_rate: u32, output_rate: u32, target: usize) -> Self {
        let cutoff = (output_rate as f64 / input_rate as f64).min(1.0) * 0.90;
        let mut coefficients = Box::new([[0.0; TAPS]; PHASES + 1]);
        for (phase, row) in coefficients.iter_mut().enumerate() {
            let fraction = phase as f64 / PHASES as f64;
            let mut sum = 0.0;
            for (tap, value) in row.iter_mut().enumerate() {
                let x = tap as f64 - 15.0 + fraction;
                let sinc = if x.abs() < 1.0e-10 {
                    cutoff
                } else {
                    (PI * x * cutoff).sin() / (PI * x)
                };
                let window =
                    0.42 + 0.5 * (PI * x / 16.0).cos() + 0.08 * (2.0 * PI * x / 16.0).cos();
                *value = (sinc * window) as f32;
                sum += *value;
            }
            for value in row {
                *value /= sum;
            }
        }
        Self {
            coefficients,
            history: [0.0; TAPS],
            cursor: 0,
            phase: 1.0,
            ratio: input_rate as f64 / output_rate as f64,
            correction: 0.0,
            target: target.max(TAPS * 2),
            primed: false,
            underruns: 0,
            dropped: 0,
        }
    }
    pub fn begin_block(&mut self, input: &mut Consumer<f32>, output_frames: usize) -> bool {
        let required = (output_frames as f64 * self.ratio * 1.003).ceil() as usize + 2;
        let available = input.slots();
        if available > self.target * 3 + required {
            let discard = available.saturating_sub(self.target + required);
            for _ in 0..discard {
                let _ = input.pop();
            }
            self.dropped += discard as u64;
            self.history.fill(0.0);
            self.primed = false;
        }
        if !self.primed {
            if input.slots() < self.target + required {
                return false;
            }
            self.primed = true;
            self.phase = 1.0;
        }
        if input.slots() < required {
            self.underruns += 1;
            self.primed = false;
            self.history.fill(0.0);
            return false;
        }
        // Slow, bounded correction for independent device clocks, not packet jitter.
        let error =
            (input.slots() as f64 - required as f64 - self.target as f64) / self.target as f64;
        let desired = (error * 0.0005).clamp(-0.002, 0.002);
        self.correction += 0.01 * (desired - self.correction);
        true
    }
    pub fn next(&mut self, input: &mut Consumer<f32>) -> f32 {
        while self.phase >= 1.0 {
            match input.pop() {
                Ok(sample) => {
                    self.history[self.cursor] = sample;
                    self.cursor = (self.cursor + 1) % TAPS;
                }
                Err(_) => {
                    self.primed = false;
                    self.underruns += 1;
                    return 0.0;
                }
            }
            self.phase -= 1.0;
        }
        let position = self.phase * PHASES as f64;
        let index = (position as usize).min(PHASES - 1);
        let fraction = (position - index as f64) as f32;
        let mut sample = 0.0;
        for tap in 0..TAPS {
            let coefficient = self.coefficients[index][tap]
                + fraction * (self.coefficients[index + 1][tap] - self.coefficients[index][tap]);
            sample += self.history[(self.cursor + TAPS - 1 - tap) % TAPS] * coefficient;
        }
        self.phase += self.ratio * (1.0 + self.correction);
        sample
    }
}
