pub(crate) trait EncoderManagerStateSpec {
    type VideoEncoderState;
}

pub(crate) trait EncoderManager {
    type State: EncoderManagerStateSpec;

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>;
}
