use super::DecodedVideoFrame;

pub(crate) trait VideoDecoder {
    type DecoderInput;
    type DecodedBuffer;

    fn decode(
        &mut self,
        input: Self::DecoderInput,
    ) -> eros::Result<DecodedVideoFrame<Self::DecodedBuffer>>;
}
