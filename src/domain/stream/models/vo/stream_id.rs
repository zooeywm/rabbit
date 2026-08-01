#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(u16);

impl StreamId {
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    pub fn value(self) -> u16 {
        self.0
    }
}
