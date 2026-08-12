use crate::app::container::stream_pipeline::outbound_port::EncodedVideoFrame;

pub(crate) trait Packetizer {
    type EncodedBuffer;

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()>;
}
