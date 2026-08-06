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
    encoder_frame_converter_state: CvtSt,

    /// Encodes converted inputs into compressed video frames.
    video_encoder_state: EcdSt,
}

impl<CvtSt, EcdSt> StreamPipelineContainer<CvtSt, EcdSt> {
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
