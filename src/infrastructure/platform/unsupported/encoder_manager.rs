use crate::app::container::root::outbound_port::{EncoderManager, EncoderManagerStateSpec};

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

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
