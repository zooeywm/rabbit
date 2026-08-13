use crate::{
    app::container::{
        client::outbound_port::VideoDecoderState,
        client_stream_pipeline::outbound_port::{DecodedVideoFrame, VideoDecodeUnit, VideoDecoder},
    },
    domain::stream::models::vo::FrameId,
};

pub(crate) struct FakeDecoderInput {
    pub(crate) frame_id: FrameId,
    pub(crate) buffer: [u8; 8],
    recovery_point: bool,
}

impl FakeDecoderInput {
    pub(crate) fn new(frame_id: FrameId, buffer: [u8; 8], recovery_point: bool) -> Self {
        Self {
            frame_id,
            buffer,
            recovery_point,
        }
    }
}

impl VideoDecodeUnit for FakeDecoderInput {
    fn is_recovery_point(&self) -> bool {
        self.recovery_point
    }
}

#[derive(kudi::DepInj)]
#[target(FakeVideoDecoderImpl)]
pub(crate) struct FakeVideoDecoderState {
    output: Option<DecodedVideoFrame<[u8; 8]>>,
}

impl FakeVideoDecoderState {
    pub(crate) fn new() -> Self {
        Self { output: None }
    }
}

impl VideoDecoderState for FakeVideoDecoderState {
    fn new() -> eros::Result<Self> {
        Ok(Self::new())
    }
}

impl<Deps> VideoDecoder for FakeVideoDecoderImpl<Deps>
where
    Deps: AsMut<FakeVideoDecoderState>,
{
    type DecoderInput = FakeDecoderInput;
    type DecodedBuffer = [u8; 8];

    fn reset(&mut self) -> eros::Result<()> {
        self.prj_ref_mut().as_mut().output = None;
        Ok(())
    }

    fn submit(&mut self, input: Self::DecoderInput) -> eros::Result<()> {
        let state = self.prj_ref_mut().as_mut();
        if state.output.is_some() {
            eros::bail!("Fake decoder output must be received before submitting another unit");
        }
        state.output = Some(DecodedVideoFrame::new(input.frame_id, input.buffer));
        Ok(())
    }

    fn try_receive(&mut self) -> eros::Result<Option<DecodedVideoFrame<Self::DecodedBuffer>>> {
        Ok(self.prj_ref_mut().as_mut().output.take())
    }
}
