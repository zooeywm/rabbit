use crate::app::container::root::outbound_port::EncoderManager;

use super::encoder::VideoEncoderState;

#[derive(kudi::DepInj)]
#[target(EncoderManagerImpl)]
pub(crate) struct EncoderManagerState;

impl EncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}

impl<Deps> EncoderManager for EncoderManagerImpl<Deps> {
    type VideoEncoderState = VideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}
