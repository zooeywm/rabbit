use std::time::Duration;

use crate::{
    app::container::ResourceUsage,
    domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
};

pub(crate) trait NetworkMetricsRecorder {
    fn register_network_queue_usage(&self, usage: ResourceUsage);

    fn unregister_network_queue_usage(&self);

    /// Records packetization completion for a source capture frame.
    fn record_packetized_frame(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        frame_id: FrameId,
        duration: Duration,
    );

    fn record_sent_bytes(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    );
}
