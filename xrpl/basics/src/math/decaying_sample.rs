//! Parity surface for `xrpl/basics/DecayingSample.h`.

use std::time::Instant;

/// Clock-like boundary used by the decaying sample helpers.
pub trait DecayTimePoint: Copy + Eq + Ord {
    fn elapsed_seconds_since(self, earlier: Self) -> u64;
    fn elapsed_seconds_f64_since(self, earlier: Self) -> f64;
}

impl DecayTimePoint for Instant {
    fn elapsed_seconds_since(self, earlier: Self) -> u64 {
        self.duration_since(earlier).as_secs()
    }

    fn elapsed_seconds_f64_since(self, earlier: Self) -> f64 {
        self.duration_since(earlier).as_secs_f64()
    }
}

/// Sampling function using exponential decay to provide a continuous value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecayingSample<const WINDOW: i64, T = Instant>
where
    T: DecayTimePoint,
{
    value: i64,
    when: T,
}

impl<const WINDOW: i64, T> DecayingSample<WINDOW, T>
where
    T: DecayTimePoint,
{
    pub fn new(now: T) -> Self {
        Self {
            value: 0,
            when: now,
        }
    }

    pub fn add(&mut self, value: i64, now: T) -> i64 {
        self.decay(now);
        self.value += value;
        self.value / WINDOW
    }

    pub fn value(&mut self, now: T) -> i64 {
        self.decay(now);
        self.value / WINDOW
    }

    fn decay(&mut self, now: T) {
        if now == self.when {
            return;
        }

        if self.value != 0 {
            let mut elapsed = now.elapsed_seconds_since(self.when) as i64;
            if elapsed > 4 * WINDOW {
                self.value = 0;
            } else {
                while elapsed > 0 {
                    self.value -= (self.value + WINDOW - 1) / WINDOW;
                    elapsed -= 1;
                }
            }
        }

        self.when = now;
    }
}

/// Sampling function using exponential decay with a fixed half-life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecayWindow<const HALF_LIFE: i64, T = Instant>
where
    T: DecayTimePoint,
{
    value: f64,
    when: T,
}

impl<const HALF_LIFE: i64, T> DecayWindow<HALF_LIFE, T>
where
    T: DecayTimePoint,
{
    pub fn new(now: T) -> Self {
        assert!(HALF_LIFE > 0, "half life must be positive");
        Self {
            value: 0.0,
            when: now,
        }
    }

    pub fn add(&mut self, value: f64, now: T) {
        self.decay(now);
        self.value += value;
    }

    pub fn value(&mut self, now: T) -> f64 {
        self.decay(now);
        self.value / HALF_LIFE as f64
    }

    fn decay(&mut self, now: T) {
        if now <= self.when {
            return;
        }

        let elapsed = now.elapsed_seconds_f64_since(self.when);
        self.value *= f64::powf(2.0, -elapsed / HALF_LIFE as f64);
        self.when = now;
    }
}

#[cfg(test)]
mod tests {
    use super::{DecayWindow, DecayingSample};
    use std::time::{Duration, Instant};

    #[test]
    fn decaying_sample_decay_shape() {
        let start = Instant::now();
        let mut sample = DecayingSample::<32>::new(start);
        assert_eq!(sample.add(3200, start), 100);
        assert_eq!(sample.value(start + Duration::from_secs(1)), 96);
        assert_eq!(sample.value(start + Duration::from_secs(2)), 93);
    }

    #[test]
    fn decaying_sample_resets_after_four_windows() {
        let start = Instant::now();
        let mut sample = DecayingSample::<8>::new(start);
        assert_eq!(sample.add(800, start), 100);
        assert_eq!(sample.value(start + Duration::from_secs(33)), 0);
    }

    #[test]
    fn decay_window_half_life_shape() {
        let start = Instant::now();
        let mut window = DecayWindow::<10>::new(start);
        window.add(20.0, start);
        let normalized = window.value(start + Duration::from_secs(10));
        assert!((normalized - 1.0).abs() < 1e-9);
    }
}
