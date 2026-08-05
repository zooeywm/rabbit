mod inbound;
mod latest_frame_slot;
mod model;

pub(crate) mod outbound_port;

pub(crate) use latest_frame_slot::LatestFrameSlot;
pub(crate) use model::EncodedVideoFrame;

/// A dynamically created container for one stream pipeline.
///
/// The container will coordinate frame conversion, encoding, packetization,
/// transport, and their runtime scheduling.
pub(crate) struct StreamPipelineContainer;
