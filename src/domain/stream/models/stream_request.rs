use super::vo::{CaptureSourceId, StreamId};

#[derive(Clone, Copy)]
pub(crate) enum StreamRequest {
    Start {
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    },
    Remove {
        stream_id: StreamId,
    },
}
