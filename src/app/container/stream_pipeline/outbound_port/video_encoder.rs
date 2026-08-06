use crate::app::container::stream_pipeline::model::EncodedVideoFrame;

pub(crate) trait VideoEncoder {
    type EncoderInput;

    type EncodedBuffer;

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>>;
}
