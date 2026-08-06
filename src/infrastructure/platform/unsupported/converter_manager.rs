use crate::app::container::root::outbound_port::{ConverterManager, ConverterManagerStateSpec};

#[derive(kudi::DepInj)]
#[target(UnsupportedConverterManagerImpl)]
pub(crate) struct UnsupportedConverterManagerState;

impl UnsupportedConverterManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> ConverterManager for UnsupportedConverterManagerImpl<Deps> {
    type State = UnsupportedConverterManagerState;

    fn create_encoder_frame_converter(
        &mut self,
    ) -> eros::Result<<Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
