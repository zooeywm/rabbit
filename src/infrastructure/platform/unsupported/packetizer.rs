use std::convert::Infallible;

use crate::app::container::{
    packetization::outbound_port::Packetizer, stream_pipeline::outbound_port::EncodedVideoFrame,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedPacketizerImpl)]
pub(crate) struct UnsupportedPacketizerState {
    never: Infallible,
}

impl<Deps> Packetizer for UnsupportedPacketizerImpl<Deps> {
    type EncodedBuffer = Infallible;

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        match frame.data {}
    }
}
