use crate::app::container::stream_pipeline::model::EncodedVideoFrame;

/// Encodes converted frame input into compressed video data.
pub(crate) trait VideoEncoder {
    /// The input type accepted by this encoder.
    type EncoderInput;

    /// The encoded buffer type produced by this encoder.
    type EncodedBuffer;

    /// Encodes one frame.
    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>>;
}
