use crate::app::container::root::outbound_port::{ConverterManager, ConverterManagerStateSpec};

#[derive(kudi::DepInj)]
#[target(LinuxConverterManagerImpl)]
pub(crate) struct LinuxConverterManagerState;

impl LinuxConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux frame conversion infrastructure has not been implemented");
    }
}

impl<Deps> ConverterManager for LinuxConverterManagerImpl<Deps> {
    type State = LinuxConverterManagerState;

    fn create_encoder_frame_converter(
        &mut self,
    ) -> eros::Result<<Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState> {
        eros::bail!("Linux frame conversion infrastructure has not been implemented");
    }
}
