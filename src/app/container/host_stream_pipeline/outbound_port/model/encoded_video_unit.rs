use crate::app::container::host_stream_pipeline::outbound_port::UnitNumber;
use crate::domain::stream::models::vo::FrameId;

pub struct EncodedVideoUnit<Buffer> {
    /// Correlates this encoded unit with its source capture frame for metrics only.
    pub source_frame_id: FrameId,
    pub unit_number: UnitNumber,
    pub is_keyframe: bool,
    pub data: Buffer,
}

impl<Buffer> EncodedVideoUnit<Buffer> {
    pub fn new(
        source_frame_id: FrameId,
        unit_number: UnitNumber,
        is_keyframe: bool,
        data: Buffer,
    ) -> Self {
        Self {
            source_frame_id,
            unit_number,
            is_keyframe,
            data,
        }
    }
}
