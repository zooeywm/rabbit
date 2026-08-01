#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureSourceId(u16);

impl CaptureSourceId {
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    pub fn value(self) -> u16 {
        self.0
    }
}
