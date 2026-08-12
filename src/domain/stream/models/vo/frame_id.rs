use super::capture_source_id::CaptureSourceId;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId {
    capture_source_id: CaptureSourceId,
    sequence: u64,
}

impl FrameId {
    pub fn new(capture_source_id: CaptureSourceId, sequence: u64) -> Self {
        Self {
            capture_source_id,
            sequence,
        }
    }

    pub fn capture_source_id(self) -> CaptureSourceId {
        self.capture_source_id
    }
}
