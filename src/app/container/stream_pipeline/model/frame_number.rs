#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FrameNumber(u64);

impl FrameNumber {
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}
