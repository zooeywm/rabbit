/// Describes the encoder state created by one encoder-manager state.
pub(crate) trait EncoderManagerStateSpec {
    type VideoEncoderState;
}

/// Creates video-encoder states for stream-pipeline containers.
///
/// This port only describes dependency creation. The stream-pipeline
/// container determines how the created encoder is scheduled and destroyed.
pub(crate) trait EncoderManager {
    /// The video-encoder state created by this manager.
    type VideoEncoderState;

    /// Creates one video-encoder state.
    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState>;
}
