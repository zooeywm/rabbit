pub(crate) mod inbound;
pub(crate) mod outbound_port;

pub(crate) struct StreamPipelineContainer<CvtSt, EcdSt> {
    encoder_frame_converter_state: CvtSt,
    video_encoder_state: EcdSt,
}

impl<CvtSt, EcdSt> StreamPipelineContainer<CvtSt, EcdSt> {
    pub(crate) fn new(encoder_frame_converter_state: CvtSt, video_encoder_state: EcdSt) -> Self {
        Self {
            encoder_frame_converter_state,
            video_encoder_state,
        }
    }

    pub(crate) fn encoder_frame_converter_state(&self) -> &CvtSt {
        &self.encoder_frame_converter_state
    }

    pub(crate) fn encoder_frame_converter_state_mut(&mut self) -> &mut CvtSt {
        &mut self.encoder_frame_converter_state
    }

    pub(crate) fn video_encoder_state(&self) -> &EcdSt {
        &self.video_encoder_state
    }

    pub(crate) fn video_encoder_state_mut(&mut self) -> &mut EcdSt {
        &mut self.video_encoder_state
    }
}
