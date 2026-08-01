use crate::domain::{
    AggregateRoot,
    stream::models::vo::{CaptureSourceId, StreamId, StreamStatus},
};

pub struct Stream {
    id: StreamId,
    capture_source_id: CaptureSourceId,
    status: StreamStatus,
}

impl Stream {
    pub fn new(id: StreamId, capture_source_id: CaptureSourceId) -> Self {
        Self {
            id,
            capture_source_id,
            status: StreamStatus::Stopped,
        }
    }

    pub fn capture_source_id(&self) -> CaptureSourceId {
        self.capture_source_id
    }

    pub fn status(&self) -> StreamStatus {
        self.status
    }

    pub fn start(&mut self) {
        self.status = StreamStatus::Running;
    }

    pub fn stop(&mut self) {
        self.status = StreamStatus::Stopped;
    }
}

impl AggregateRoot for Stream {
    type Id = StreamId;

    fn id(&self) -> Self::Id {
        self.id
    }
}
