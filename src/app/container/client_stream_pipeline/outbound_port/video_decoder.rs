use super::DecodedVideoFrame;

pub(crate) trait VideoDecoder {
    type DecoderInput;
    type DecodedBuffer;

    fn reset(&mut self) -> eros::Result<()>;

    fn submit(&mut self, input: Self::DecoderInput) -> eros::Result<()>;

    fn try_receive(&mut self) -> eros::Result<Option<DecodedVideoFrame<Self::DecodedBuffer>>>;
}

impl VideoDecoder for super::super::ClientStreamPipelineContainer<()> {
    type DecoderInput = std::convert::Infallible;
    type DecodedBuffer = std::convert::Infallible;

    fn reset(&mut self) -> eros::Result<()> {
        Ok(())
    }

    fn submit(&mut self, input: Self::DecoderInput) -> eros::Result<()> {
        match input {}
    }

    fn try_receive(&mut self) -> eros::Result<Option<DecodedVideoFrame<Self::DecodedBuffer>>> {
        Ok(None)
    }
}
