use std::convert::Infallible;

use crate::app::container::host::outbound_port::EncoderFrameConverterState;

pub(crate) struct LinuxEncoderFrameConverterState {
    never: Infallible,
}

impl EncoderFrameConverterState for LinuxEncoderFrameConverterState {
    fn new() -> eros::Result<Self> {
        eros::bail!("Linux frame conversion infrastructure has not been implemented")
    }
}
