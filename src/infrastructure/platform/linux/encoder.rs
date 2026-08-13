use std::convert::Infallible;

use crate::app::container::host::outbound_port::VideoEncoderState;

pub(crate) struct LinuxVideoEncoderState {
    never: Infallible,
}

impl VideoEncoderState for LinuxVideoEncoderState {
    fn new() -> eros::Result<Self> {
        eros::bail!("Linux video encoding infrastructure has not been implemented")
    }
}
