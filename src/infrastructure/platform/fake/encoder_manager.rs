use crate::{
    app::container::app_container::outbound_port::{EncoderManager, EncoderManagerStateSpec},
    infrastructure::fake::encoder::FakeVideoEncoderState,
};

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

    fn video_encoder_state_factory(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<Deps> {
        || Ok(FakeVideoEncoderState::new())
    }
}
