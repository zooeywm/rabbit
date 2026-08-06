use crate::app::container::root::outbound_port::{EncoderManager, EncoderManagerStateSpec};

#[derive(kudi::DepInj)]
#[target(LinuxEncoderManagerImpl)]
pub(crate) struct LinuxEncoderManagerState;

impl LinuxEncoderManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}

impl<Deps> EncoderManager for LinuxEncoderManagerImpl<Deps> {
    type State = LinuxEncoderManagerState;

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState> {
        eros::bail!("Linux video encoding infrastructure has not been implemented");
    }
}
