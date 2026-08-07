use crate::{
    app::container::app_container::outbound_port::{ConverterManager, ConverterManagerStateSpec},
    infrastructure::fake::converter::FakeEncoderFrameConverterState,
};

#[derive(kudi::DepInj)]
#[target(FakeConverterManagerImpl)]
pub(crate) struct FakeConverterManagerState;

impl FakeConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> ConverterManager for FakeConverterManagerImpl<Deps> {
    type State = FakeConverterManagerState;

    fn encoder_frame_converter_state_factory(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<Deps> {
        || Ok(FakeEncoderFrameConverterState::new())
    }
}
