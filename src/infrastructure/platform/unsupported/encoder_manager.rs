use crate::app::container::app_container::outbound_port::{
    EncoderManager, EncoderManagerStateSpec,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedEncoderManagerImpl)]
pub(crate) struct UnsupportedEncoderManagerState;

impl UnsupportedEncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> EncoderManager for UnsupportedEncoderManagerImpl<Deps> {
    type State = UnsupportedEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
