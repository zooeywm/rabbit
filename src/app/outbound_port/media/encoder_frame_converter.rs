pub trait EncoderFrameConverter {
    type CapturedFrame;
    type EncoderInput;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput>;
}
