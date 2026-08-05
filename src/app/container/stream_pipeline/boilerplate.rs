use crate::infrastructure::platform::{EncoderFrameConverterState, VideoEncoderState};

use super::StreamPipelineContainer;

impl AsRef<EncoderFrameConverterState> for StreamPipelineContainer {
    fn as_ref(&self) -> &EncoderFrameConverterState {
        &self.encoder_frame_converter_state
    }
}

impl AsMut<EncoderFrameConverterState> for StreamPipelineContainer {
    fn as_mut(&mut self) -> &mut EncoderFrameConverterState {
        &mut self.encoder_frame_converter_state
    }
}

impl AsRef<VideoEncoderState> for StreamPipelineContainer {
    fn as_ref(&self) -> &VideoEncoderState {
        &self.video_encoder_state
    }
}

impl AsMut<VideoEncoderState> for StreamPipelineContainer {
    fn as_mut(&mut self) -> &mut VideoEncoderState {
        &mut self.video_encoder_state
    }
}
