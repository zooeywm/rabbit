pub(crate) mod outbound_port;

pub(crate) struct ClientStreamPipelineContainer<DcdSt> {
    video_decoder_state: DcdSt,
}

impl<DcdSt> ClientStreamPipelineContainer<DcdSt> {
    pub(crate) fn new(video_decoder_state: DcdSt) -> Self {
        Self {
            video_decoder_state,
        }
    }

    pub(crate) fn video_decoder_state(&self) -> &DcdSt {
        &self.video_decoder_state
    }

    pub(crate) fn video_decoder_state_mut(&mut self) -> &mut DcdSt {
        &mut self.video_decoder_state
    }
}
