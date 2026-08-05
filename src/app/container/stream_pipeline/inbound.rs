use crate::infrastructure::platform::{EncoderFrameConverterState, VideoEncoderState};

use super::StreamPipelineContainer;

impl StreamPipelineContainer {
    pub(crate) fn new(
        encoder_frame_converter_state: EncoderFrameConverterState,
        video_encoder_state: VideoEncoderState,
    ) -> Self {
        Self {
            encoder_frame_converter_state,
            video_encoder_state,
        }
    }
}
