#[derive(kudi::DepInj)]
#[target(FakeEncoderFrameConverterImpl)]
pub(crate) struct FakeEncoderFrameConverterState;

impl FakeEncoderFrameConverterState {
    pub(crate) fn new() -> Self {
        Self
    }
}
