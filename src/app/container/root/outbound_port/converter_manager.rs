/// Creates frame-converter states for stream-pipeline containers.
///
/// This port only describes dependency creation. The stream-pipeline
/// container determines how the created converter is scheduled and destroyed.
pub(crate) trait ConverterManager {
    /// The frame-converter state created by this manager.
    type EncoderFrameConverterState;

    /// Creates one frame-converter state.
    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState>;
}
