use crate::domain::stream::models::vo::FrameId;

pub(crate) struct DecodedVideoFrame<Buffer> {
    pub frame_id: FrameId,
    pub buffer: Buffer,
}

impl<Buffer> DecodedVideoFrame<Buffer> {
    pub(crate) fn new(frame_id: FrameId, buffer: Buffer) -> Self {
        Self { frame_id, buffer }
    }
}
