use crate::app::outbound_port::FrameNumber;

pub struct EncodedVideoFrame<Buffer> {
    pub frame_number: FrameNumber,
    pub is_keyframe: bool,
    pub data: Buffer,
}

impl<B> EncodedVideoFrame<B> {
    pub fn new(frame_number: FrameNumber, is_keyframe: bool, data: B) -> Self {
        Self {
            frame_number,
            is_keyframe,
            data,
        }
    }
}
