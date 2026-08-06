use crate::app::container::root::outbound_port::EncoderManager;

use super::encoder::LinuxVideoEncoderState;

#[derive(kudi::DepInj)]
#[target(LinuxEncoderManagerImpl)]
pub(crate) struct LinuxEncoderManagerState;

impl LinuxEncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}

impl<Deps> EncoderManager for LinuxEncoderManagerImpl<Deps> {
    type VideoEncoderState = LinuxVideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}
