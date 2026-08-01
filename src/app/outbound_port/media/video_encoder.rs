use crate::app::outbound_port::{EncodedVideoFrame, FrameNumber};

pub trait VideoEncoder {
    type EncoderInput;
    type EncodedBuffer;

    fn encode(
        &mut self,
        frame_number: FrameNumber,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>>;
}
