use std::time::Duration;

use crate::domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MetricsTarget {
    CaptureSource(CaptureSourceId),
    Stream {
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    },
}

pub(crate) trait MetricsRecorder {
    fn register_metrics_target(&self);

    fn unregister_metrics_target(&self);

    /// Records completion of one source capture frame.
    fn record_captured_frame(&self, frame_id: FrameId, duration: Duration);

    /// Records conversion completion for a source capture frame.
    fn record_converted_frame(&self, frame_id: FrameId, duration: Duration);

    /// Records encoding completion for a source capture frame.
    fn record_encoded_frame(&self, frame_id: FrameId, duration: Duration);

    /// Records packetization completion for a source capture frame.
    fn record_packetized_frame(&self, frame_id: FrameId, duration: Duration);
}
