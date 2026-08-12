use std::convert::Infallible;

use crate::app::container::{
    packetization::outbound_port::Packetizer, stream_pipeline::outbound_port::EncodedVideoFrame,
};

#[derive(kudi::DepInj)]
#[target(LinuxPacketizerImpl)]
pub(crate) struct LinuxPacketizerState {
    never: Infallible,
}

impl<Deps> Packetizer for LinuxPacketizerImpl<Deps> {
    type EncodedBuffer = Infallible;

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        match frame.data {}
    }
}
