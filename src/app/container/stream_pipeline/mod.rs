mod inbound;
mod latest_frame_slot;
mod model;

pub(crate) mod outbound_port;

pub(crate) use latest_frame_slot::LatestFrameSlot;
pub(crate) use model::{EncodedVideoFrame, FrameNumber};

/// A dynamically created container for one stream pipeline.
///
/// This container owns the converter and encoder instances associated with
/// one stream. Its inbound implementation determines their runtime scheduling.
pub(crate) struct StreamPipelineContainer<CvtSt, EcdSt> {
    /// Converts captured frames into encoder-compatible inputs.
    pub(super) encoder_frame_converter_state: CvtSt,

    /// Encodes converted inputs into compressed video frames.
    pub(super) video_encoder_state: EcdSt,
}
