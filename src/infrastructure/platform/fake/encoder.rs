#[derive(kudi::DepInj)]
#[target(FakeVideoEncoderImpl)]
pub(crate) struct FakeVideoEncoderState;

impl FakeVideoEncoderState {
    pub(crate) fn new() -> Self {
        Self
    }
}
