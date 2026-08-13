use std::convert::Infallible;

use crate::app::container::host::outbound_port::VideoEncoderState;

pub(crate) struct UnsupportedVideoEncoderState {
    never: Infallible,
}

impl VideoEncoderState for UnsupportedVideoEncoderState {
    fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
