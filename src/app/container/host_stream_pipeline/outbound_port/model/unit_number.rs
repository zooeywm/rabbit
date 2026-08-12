/// Sequence number of an encoded Host video unit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct UnitNumber(u64);

impl UnitNumber {
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}
