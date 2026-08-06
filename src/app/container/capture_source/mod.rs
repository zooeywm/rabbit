mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::domain::stream::models::vo::StreamId;

use super::stream_pipeline::StreamPipelineContainer;

/// A dynamically created container for one physical capture source.
///
/// This container owns one screen capturer and every stream pipeline consuming
/// frames from that capturer.
pub(crate) struct CaptureSourceContainer<CapSt, CvtSt, EcdSt> {
    /// The platform screen capturer state owned by this container.
    screen_capturer_state: CapSt,

    /// Owns all stream pipelines consuming this capture source.
    stream_pipelines: HashMap<StreamId, StreamPipelineContainer<CvtSt, EcdSt>>,
}

impl<CapSt, CvtSt, EcdSt> CaptureSourceContainer<CapSt, CvtSt, EcdSt> {
    pub(crate) fn screen_capturer_state(&self) -> &CapSt {
        &self.screen_capturer_state
    }

    pub(crate) fn screen_capturer_state_mut(&mut self) -> &mut CapSt {
        &mut self.screen_capturer_state
    }
}
