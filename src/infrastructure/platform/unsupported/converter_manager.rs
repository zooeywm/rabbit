use crate::app::container::app_container::outbound_port::{
    ConverterManager, ConverterManagerStateSpec,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedConverterManagerImpl)]
pub(crate) struct UnsupportedConverterManagerState;

impl UnsupportedConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> ConverterManager for UnsupportedConverterManagerImpl<Deps> {
    type State = UnsupportedConverterManagerState;

    fn compose_encoder_frame_converter_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
