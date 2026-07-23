#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }

        Some(Self {
            numerator,
            denominator,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

#[derive(Debug, Default)]
pub struct FrameRateGate {
    rates: Option<(FrameRate, FrameRate)>,
    phase: u128,
}

impl FrameRateGate {
    pub fn should_emit(&mut self, source: FrameRate, target: FrameRate) -> bool {
        let increment = u128::from(target.numerator) * u128::from(source.denominator);
        let threshold = u128::from(source.numerator) * u128::from(target.denominator);

        if self.rates != Some((source, target)) {
            self.rates = Some((source, target));
            self.phase = threshold.saturating_sub(increment);
        }

        if increment >= threshold {
            return true;
        }

        self.phase += increment;
        if self.phase < threshold {
            return false;
        }

        self.phase -= threshold;
        true
    }

    pub fn reset(&mut self) {
        self.rates = None;
        self.phase = 0;
    }
}

// Focused test: cargo test kernel::geometry::tests --lib
#[cfg(test)]
mod tests {
    use crate::kernel::geometry::{FrameRate, FrameRateGate};

    #[test]
    fn frame_rate_requires_positive_terms() {
        assert!(FrameRate::new(120, 1).is_some());
        assert!(FrameRate::new(0, 1).is_none());
        assert!(FrameRate::new(120, 0).is_none());
    }

    #[test]
    fn frame_rate_gate_reduces_source_events_to_the_target_average() {
        let source = FrameRate::new(144, 1).expect("Source frame rate should be valid");
        let target = FrameRate::new(60, 1).expect("Target frame rate should be valid");
        let mut gate = FrameRateGate::default();

        let emitted = (0..144)
            .filter(|_| gate.should_emit(source, target))
            .count();

        assert_eq!(emitted, 60);
    }

    #[test]
    fn frame_rate_gate_does_not_duplicate_frames_above_the_source_rate() {
        let source = FrameRate::new(60, 1).expect("Source frame rate should be valid");
        let target = FrameRate::new(120, 1).expect("Target frame rate should be valid");
        let mut gate = FrameRateGate::default();

        assert!((0..60).all(|_| gate.should_emit(source, target)));
    }

    #[test]
    fn frame_rate_gate_emits_immediately_after_rates_change_or_reset() {
        let source = FrameRate::new(144, 1).expect("Source frame rate should be valid");
        let sixty = FrameRate::new(60, 1).expect("Target frame rate should be valid");
        let thirty = FrameRate::new(30, 1).expect("Target frame rate should be valid");
        let mut gate = FrameRateGate::default();

        assert!(gate.should_emit(source, sixty));
        assert!(gate.should_emit(source, thirty));
        assert!(!gate.should_emit(source, thirty));

        gate.reset();
        assert!(gate.should_emit(source, thirty));
    }
}
