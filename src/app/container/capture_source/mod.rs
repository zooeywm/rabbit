mod boilerplate;
mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{domain::stream::models::vo::StreamId, infrastructure::platform::ScreenCapturerState};

use super::stream_pipeline::StreamPipelineContainer;

/// A dynamically created container for one physical capture source.
///
/// This container owns one screen capturer and every stream pipeline consuming
/// frames from that capturer.
pub(crate) struct CaptureSourceContainer {
    /// The platform screen capturer state owned by this container.
    screen_capturer_state: ScreenCapturerState,

    /// Owns all stream pipelines consuming this capture source.
    stream_pipelines: HashMap<StreamId, StreamPipelineContainer>,
}
