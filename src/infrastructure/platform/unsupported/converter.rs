use std::convert::Infallible;

use crate::app::container::host::outbound_port::EncoderFrameConverterState;

pub(crate) struct UnsupportedEncoderFrameConverterState {
    never: Infallible,
}

impl EncoderFrameConverterState for UnsupportedEncoderFrameConverterState {
    fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
