/// Fixed-timestep accumulator: the simulation advances in whole ticks while rendering runs at
/// any rate, interpolating with [`FixedClock::alpha`].
pub struct FixedClock {
    tick_seconds: f64,
    accumulator: f64,
}

impl FixedClock {
    pub fn new(ticks_per_second: f64) -> Self {
        Self { tick_seconds: 1.0 / ticks_per_second, accumulator: 0.0 }
    }

    pub fn tick_seconds(&self) -> f64 {
        self.tick_seconds
    }

    /// Add elapsed real time and return how many ticks to simulate. Long stalls are clamped so
    /// the game never tries to catch up on more than a quarter second.
    pub fn advance(&mut self, elapsed: f64) -> u32 {
        self.accumulator = (self.accumulator + elapsed.min(0.25)).max(0.0);
        let ticks = (self.accumulator / self.tick_seconds) as u32;
        self.accumulator -= ticks as f64 * self.tick_seconds;
        ticks
    }

    /// Fraction of the next tick already elapsed, for render interpolation.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.tick_seconds) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_partial_ticks() {
        let mut clock = FixedClock::new(10.0);
        assert_eq!(clock.advance(0.05), 0);
        assert_eq!(clock.advance(0.06), 1);
        assert!((clock.alpha() - 0.1).abs() < 1e-4);
    }

    #[test]
    fn clamps_long_stalls() {
        let mut clock = FixedClock::new(100.0);
        assert_eq!(clock.advance(5.0), 25);
    }
}
