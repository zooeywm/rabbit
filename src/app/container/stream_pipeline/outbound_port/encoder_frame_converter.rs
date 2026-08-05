/// Converts a captured frame into an encoder-compatible input.
///
/// This port does not determine whether conversion and encoding run on the
/// same thread or on separate execution contexts.
pub(crate) trait EncoderFrameConverter {
    /// The captured frame accepted by this converter.
    type CapturedFrame;

    /// The input type produced for the encoder.
    type EncoderInput;

    /// Converts one captured frame into one encoder input.
    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput>;
}
