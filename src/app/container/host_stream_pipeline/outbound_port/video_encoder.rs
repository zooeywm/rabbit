use crate::app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit;

pub(crate) trait VideoEncoder {
    type EncoderInput;

    type EncodedBuffer;

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoUnit<Self::EncodedBuffer>>;
}
