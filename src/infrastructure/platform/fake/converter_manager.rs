use crate::app::container::root::outbound_port::ConverterManager;

use super::converter::FakeEncoderFrameConverterState;

#[derive(kudi::DepInj)]
#[target(FakeConverterManagerImpl)]
pub(crate) struct FakeConverterManagerState;

impl FakeConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> ConverterManager for FakeConverterManagerImpl<Deps> {
    type EncoderFrameConverterState = FakeEncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState> {
        Ok(FakeEncoderFrameConverterState::new())
    }
}
