use crate::app::container::root::outbound_port::{EncoderManager, EncoderManagerStateSpec};

use super::encoder::FakeVideoEncoderState;

#[derive(kudi::DepInj)]
#[target(FakeEncoderManagerImpl)]
pub(crate) struct FakeEncoderManagerState;

impl FakeEncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> EncoderManager for FakeEncoderManagerImpl<Deps> {
    type State = FakeEncoderManagerState;

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState> {
        Ok(FakeVideoEncoderState::new())
    }
}
