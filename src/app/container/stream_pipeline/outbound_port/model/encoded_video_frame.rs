use crate::app::container::stream_pipeline::outbound_port::FrameNumber;
use crate::domain::stream::models::vo::FrameId;

pub struct EncodedVideoFrame<Buffer> {
    pub frame_id: FrameId,
    pub frame_number: FrameNumber,
    pub is_keyframe: bool,
    pub data: Buffer,
}

impl<B> EncodedVideoFrame<B> {
    pub fn new(frame_id: FrameId, frame_number: FrameNumber, is_keyframe: bool, data: B) -> Self {
        Self {
            frame_id,
            frame_number,
            is_keyframe,
            data,
        }
    }
}
