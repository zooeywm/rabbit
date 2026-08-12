use crate::{
    app::container::client::outbound_port::{DecoderManager, DecoderManagerStateSpec},
    infrastructure::fake::decoder::FakeVideoDecoderState,
};

#[derive(kudi::DepInj)]
#[target(FakeDecoderManagerImpl)]
pub(crate) struct FakeDecoderManagerState;

impl FakeDecoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> DecoderManager for FakeDecoderManagerImpl<Deps> {
    type State = FakeDecoderManagerState;

    fn compose_video_decoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as DecoderManagerStateSpec>::VideoDecoderState>
    + Send
    + 'static
    + use<Deps> {
        || Ok(FakeVideoDecoderState::new())
    }
}
