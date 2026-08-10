use std::time::Instant;

use eros::Context;

use crate::{
    app::container::{
        root::outbound_port::MetricsRecorder,
        stream_pipeline::outbound_port::{EncodedVideoFrame, FrameNumber, VideoEncoder},
    },
    infrastructure::fake::converter::FakeEncoderInput,
};

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
    Deps: AsMut<FakeVideoEncoderState> + MetricsRecorder,
{
    type EncoderInput = FakeEncoderInput;
    type EncodedBuffer = [u8; 8];

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        let encode_started_at = Instant::now();
        let state = self.prj_ref_mut().as_mut();
        let frame_number = state.next_frame_number;

        state.next_frame_number = state
            .next_frame_number
            .checked_add(1)
            .with_context(|| "Fake video encoder frame number space is exhausted")?;

        let frame = EncodedVideoFrame::new(
            FrameNumber::new(frame_number),
            frame_number == 0,
            input.capture_sequence.to_le_bytes(),
        );

        self.prj_ref()
            .record_encoded_frame(encode_started_at.elapsed());

        Ok(frame)
    }
}
