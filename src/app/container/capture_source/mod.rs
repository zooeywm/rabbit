mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::domain::stream::models::vo::StreamId;

use super::stream_pipeline::StreamPipelineContainer;

pub(crate) struct CaptureSourceContainer<CapSt, CvtSt, EcdSt> {
    screen_capturer_state: CapSt,

    stream_pipelines: HashMap<StreamId, StreamPipelineContainer<CvtSt, EcdSt>>,
}

impl<CapSt, CvtSt, EcdSt> CaptureSourceContainer<CapSt, CvtSt, EcdSt> {
    pub(crate) fn new(screen_capturer_state: CapSt) -> Self {
        Self {
            screen_capturer_state,
            stream_pipelines: HashMap::new(),
        }
    }

    pub(crate) fn screen_capturer_state(&self) -> &CapSt {
        &self.screen_capturer_state
    }

    pub(crate) fn screen_capturer_state_mut(&mut self) -> &mut CapSt {
        &mut self.screen_capturer_state
    }
}
