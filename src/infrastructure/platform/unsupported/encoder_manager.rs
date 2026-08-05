use crate::app::container::root::outbound_port::EncoderManager;

use super::encoder::VideoEncoderState;

#[derive(kudi::DepInj)]
#[target(EncoderManagerImpl)]
pub(crate) struct EncoderManagerState;

impl EncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> EncoderManager for EncoderManagerImpl<Deps> {
    type VideoEncoderState = VideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
