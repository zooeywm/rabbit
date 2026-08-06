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
    pub(super) screen_capturer_state: CapSt,

    /// Owns all stream pipelines consuming this capture source.
    pub(super) stream_pipelines: HashMap<StreamId, StreamPipelineContainer<CvtSt, EcdSt>>,
}
