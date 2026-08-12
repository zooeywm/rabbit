use crate::{
    app::container::client_stream_pipeline::outbound_port::{DecodedVideoFrame, VideoDecoder},
    domain::stream::models::vo::FrameId,
};

pub(crate) struct FakeDecoderInput {
    pub(crate) frame_id: FrameId,
    pub(crate) buffer: [u8; 8],
}

impl FakeDecoderInput {
    pub(crate) fn new(frame_id: FrameId, buffer: [u8; 8]) -> Self {
        Self { frame_id, buffer }
    }
}

#[derive(kudi::DepInj)]
#[target(FakeVideoDecoderImpl)]
pub(crate) struct FakeVideoDecoderState;

impl FakeVideoDecoderState {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl<Deps> VideoDecoder for FakeVideoDecoderImpl<Deps>
where
    Deps: AsMut<FakeVideoDecoderState>,
{
    type DecoderInput = FakeDecoderInput;
    type DecodedBuffer = [u8; 8];

    fn decode(
        &mut self,
        input: Self::DecoderInput,
    ) -> eros::Result<DecodedVideoFrame<Self::DecodedBuffer>> {
        let _state = self.prj_ref_mut().as_mut();
        Ok(DecodedVideoFrame::new(input.frame_id, input.buffer))
    }
}
