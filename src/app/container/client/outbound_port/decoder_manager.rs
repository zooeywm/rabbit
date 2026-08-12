pub(crate) trait DecoderManagerStateSpec {
    type VideoDecoderState: 'static;
}

pub(crate) trait DecoderManager {
    type State: DecoderManagerStateSpec;

    fn compose_video_decoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as DecoderManagerStateSpec>::VideoDecoderState>
    + Send
    + 'static
    + use<Self>;
}
