use std::time::Duration;

use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

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

    fn record_captured_frame(&self, duration: Duration);

    fn record_converted_frame(&self, duration: Duration);

    fn record_encoded_frame(&self, duration: Duration);
}
