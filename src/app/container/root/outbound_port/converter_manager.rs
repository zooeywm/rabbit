pub(crate) trait ConverterManagerStateSpec {
    type EncoderFrameConverterState;
}

pub(crate) trait ConverterManager {
    type EncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState>;
}
