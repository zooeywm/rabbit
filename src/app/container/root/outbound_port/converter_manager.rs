pub(crate) trait ConverterManagerStateSpec {
    type EncoderFrameConverterState;
}

pub(crate) trait ConverterManager {
    type State: ConverterManagerStateSpec;

    fn create_encoder_frame_converter(
        &mut self,
    ) -> eros::Result<<Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState>;
}
