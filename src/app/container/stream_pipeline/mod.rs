mod boilerplate;
mod inbound;
mod latest_frame_slot;
mod model;

pub(crate) mod outbound_port;

use crate::infrastructure::platform::{EncoderFrameConverterState, VideoEncoderState};

pub(crate) use latest_frame_slot::LatestFrameSlot;
pub(crate) use model::{EncodedVideoFrame, FrameNumber};

/// A dynamically created container for one stream pipeline.
///
/// This container owns the converter and encoder instances associated with
/// one stream. Its inbound implementation determines their runtime scheduling.
pub(crate) struct StreamPipelineContainer {
    /// Converts captured frames into encoder-compatible inputs.
    encoder_frame_converter_state: EncoderFrameConverterState,

    /// Encodes converted inputs into compressed video frames.
    video_encoder_state: VideoEncoderState,
}
