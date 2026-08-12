use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

pub(crate) struct SentBytes {
    pub(crate) capture_source_id: CaptureSourceId,
    pub(crate) stream_id: StreamId,
    pub(crate) bytes: usize,
}

impl SentBytes {
    pub(crate) fn new(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    ) -> Self {
        Self {
            capture_source_id,
            stream_id,
            bytes,
        }
    }
}
