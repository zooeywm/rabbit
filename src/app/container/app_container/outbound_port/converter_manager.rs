pub(crate) trait ConverterManagerStateSpec {
    type EncoderFrameConverterState: 'static;
}

pub(crate) trait ConverterManager {
    type State: ConverterManagerStateSpec;

    fn compose_encoder_frame_converter_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<Self>;
}
