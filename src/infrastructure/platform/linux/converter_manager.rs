use crate::app::container::root::outbound_port::ConverterManager;

use super::converter::LinuxEncoderFrameConverterState;

#[derive(kudi::DepInj)]
#[target(LinuxConverterManagerImpl)]
pub(crate) struct LinuxConverterManagerState;

impl LinuxConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux frame conversion infrastructure has not been implemented");
    }
}

impl<Deps> ConverterManager for LinuxConverterManagerImpl<Deps> {
    type EncoderFrameConverterState = LinuxEncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState> {
        eros::bail!("Linux frame conversion infrastructure has not been implemented");
    }
}
