pub(crate) mod inbound;
pub(crate) mod outbound_port;

use crate::{
    app::container::host::outbound_port::MetricsTarget,
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

pub(crate) struct HostStreamPipelineContainer<CvtSt, EcdSt> {
    metrics_target: MetricsTarget,
    encoder_frame_converter_state: CvtSt,
    video_encoder_state: EcdSt,
}

impl<CvtSt, EcdSt> HostStreamPipelineContainer<CvtSt, EcdSt> {
    pub(crate) fn new(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        encoder_frame_converter_state: CvtSt,
        video_encoder_state: EcdSt,
    ) -> Self {
        Self {
            metrics_target: MetricsTarget::Stream {
                capture_source_id,
                stream_id,
            },
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

impl<CvtSt, EcdSt> AsRef<MetricsTarget> for HostStreamPipelineContainer<CvtSt, EcdSt> {
    fn as_ref(&self) -> &MetricsTarget {
        &self.metrics_target
    }
}
