pub(crate) trait EncoderManagerStateSpec {
    type VideoEncoderState: 'static;
}

pub(crate) trait EncoderManager {
    type State: EncoderManagerStateSpec;

    fn video_encoder_state_factory(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<Self>;
}
