/// Converts captured frames into inputs for a Host video encoder.
pub(crate) trait EncoderFrameConverter {
    type CapturedFrame;

    type EncoderInput;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput>;
}
