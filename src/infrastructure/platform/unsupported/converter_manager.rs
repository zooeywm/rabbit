use crate::app::container::root::outbound_port::ConverterManager;

use super::converter::EncoderFrameConverterState;

#[derive(kudi::DepInj)]
#[target(ConverterManagerImpl)]
pub(crate) struct ConverterManagerState;

impl ConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> ConverterManager for ConverterManagerImpl<Deps> {
    type EncoderFrameConverterState = EncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
