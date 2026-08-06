use eros::Context;

use crate::app::container::stream_pipeline::{
    EncodedVideoFrame, FrameNumber, outbound_port::VideoEncoder,
};

use super::converter::FakeEncoderInput;

#[derive(kudi::DepInj)]
#[target(FakeVideoEncoderImpl)]
pub(crate) struct FakeVideoEncoderState {
    next_frame_number: u64,
}

impl FakeVideoEncoderState {
    pub(crate) fn new() -> Self {
        Self {
            next_frame_number: 0,
        }
    }
}

impl<Deps> VideoEncoder for FakeVideoEncoderImpl<Deps>
where
    Deps: AsMut<FakeVideoEncoderState>,
{
    type EncoderInput = FakeEncoderInput;
    type EncodedBuffer = [u8; 8];

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        let state = self.prj_ref_mut().as_mut();
        let frame_number = state.next_frame_number;

        state.next_frame_number = state
            .next_frame_number
            .checked_add(1)
            .with_context(|| "Fake video encoder frame number space is exhausted")?;

        Ok(EncodedVideoFrame::new(
            FrameNumber::new(frame_number),
            frame_number == 0,
            input.capture_sequence.to_le_bytes(),
        ))
    }
}
