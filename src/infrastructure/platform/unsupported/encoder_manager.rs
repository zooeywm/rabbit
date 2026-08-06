use crate::app::container::root::outbound_port::EncoderManager;

use super::encoder::UnsupportedVideoEncoderState;

#[derive(kudi::DepInj)]
#[target(UnsupportedEncoderManagerImpl)]
pub(crate) struct UnsupportedEncoderManagerState;

impl UnsupportedEncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> EncoderManager for UnsupportedEncoderManagerImpl<Deps> {
    type VideoEncoderState = UnsupportedVideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
