use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

// Six seconds of paired, 50 ms RMS measurements. Only the capture thread writes.
pub const POINTS: usize = 120;

#[derive(Clone, Copy, Default)]
pub struct Levels {
    pub input: f32,
    pub output: f32,
}

pub struct RmsWindow {
    frames_per_point: u32,
    frames: u32,
    input_energy: f64,
    output_energy: f64,
}

impl RmsWindow {
    pub fn new(rate: u32) -> Self {
        Self {
            frames_per_point: (rate / 20).max(1),
            frames: 0,
            input_energy: 0.0,
            output_energy: 0.0,
        }
    }

    #[inline]
    pub fn sample(&mut self, input: f32, output: f32) -> Option<Levels> {
        self.input_energy += f64::from(input).powi(2);
        self.output_energy += f64::from(output).powi(2);
        self.frames += 1;
        if self.frames < self.frames_per_point {
            return None;
        }
        let count = f64::from(self.frames);
        let levels = Levels {
            input: (self.input_energy / count).sqrt() as f32,
            output: (self.output_energy / count).sqrt() as f32,
        };
        self.frames = 0;
        self.input_energy = 0.0;
        self.output_energy = 0.0;
        Some(levels)
    }
}

pub struct LevelHistory {
    // The spare slot lets a producer publish the next pair while the UI reads
    // all POINTS of the previous history.
    slots: [AtomicU64; POINTS + 1],
    end: AtomicU64,
}

impl Default for LevelHistory {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| AtomicU64::new(0)),
            end: AtomicU64::new(0),
        }
    }
}

impl LevelHistory {
    fn publish(&self, pair: u64) {
        let index = self.end.load(SeqCst);
        self.slots[index as usize % self.slots.len()].store(pair, SeqCst);
        self.end.store(index + 1, SeqCst);
    }

    pub fn push(&self, levels: Levels) {
        self.publish(
            (u64::from(levels.input.to_bits()) << 32) | u64::from(levels.output.to_bits()),
        );
    }

    pub fn start_session(&self) {
        self.publish(u64::MAX); // Gap between devices/sessions, distinct from silence.
    }

    pub fn snapshot(&self, previous: Option<u64>) -> Option<(u64, Vec<Levels>)> {
        let end = self.end.load(SeqCst);
        if previous == Some(end) {
            return None;
        }
        let mut levels = Vec::with_capacity(POINTS);
        for index in (end.saturating_sub(POINTS as u64)..end).rev() {
            let pair = self.slots[index as usize % self.slots.len()].load(SeqCst);
            if pair == u64::MAX {
                break;
            }
            levels.push(Levels {
                input: f32::from_bits((pair >> 32) as u32),
                output: f32::from_bits(pair as u32),
            });
        }
        if self.end.load(SeqCst) != end {
            return None; // UI retries on its next tick.
        }
        levels.reverse();
        Some((end, levels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_uses_the_same_frames_for_both_signals() {
        let mut window = RmsWindow::new(80);
        for input in [1.0, 0.0, 0.0] {
            assert!(window.sample(input, 0.0).is_none());
        }
        let pair = window.sample(0.0, -0.25).unwrap();
        assert_eq!((pair.input, pair.output), (0.5, 0.125));
        for _ in 0..3 {
            assert!(window.sample(0.0, 0.0).is_none());
        }
        let pair = window.sample(0.0, 0.0).unwrap();
        assert_eq!((pair.input, pair.output), (0.0, 0.0));
    }

    #[test]
    fn history_wraps_and_starts_fresh_on_reconnect() {
        let history = LevelHistory::default();
        for i in 0..(POINTS * 3) {
            history.push(Levels {
                input: i as f32,
                output: i as f32 * 3.5,
            });
        }
        let (revision, points) = history.snapshot(None).unwrap();
        assert_eq!(points.len(), POINTS);
        for (i, pair) in points.iter().enumerate() {
            assert_eq!(pair.input, (POINTS * 2 + i) as f32);
            assert_eq!(pair.output, pair.input * 3.5);
        }
        assert!(history.snapshot(Some(revision)).is_none());
        history.start_session();
        assert!(history.snapshot(Some(revision)).unwrap().1.is_empty());
        history.push(Levels::default());
        assert_eq!(history.snapshot(None).unwrap().1.len(), 1);
    }
}
