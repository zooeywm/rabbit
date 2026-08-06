pub(crate) trait EncoderManagerStateSpec {
    type VideoEncoderState;
}

pub(crate) trait EncoderManager {
    type VideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState>;
}
