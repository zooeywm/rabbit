use super::StreamPipelineContainer;

impl<CvtSt, EcdSt> StreamPipelineContainer<CvtSt, EcdSt> {
    pub(crate) fn new(encoder_frame_converter_state: CvtSt, video_encoder_state: EcdSt) -> Self {
        Self {
            encoder_frame_converter_state,
            video_encoder_state,
        }
    }
}
