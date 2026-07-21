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

#[cfg(test)]
mod tests {
    use crate::kernel::geometry::FrameRate;

    #[test]
    fn frame_rate_requires_positive_terms() {
        assert!(FrameRate::new(120, 1).is_some());
        assert!(FrameRate::new(0, 1).is_none());
        assert!(FrameRate::new(120, 0).is_none());
    }
}
