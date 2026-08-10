use std::time::Instant;

use crate::{
    app::container::{
        root::outbound_port::MetricsRecorder, stream_pipeline::outbound_port::EncoderFrameConverter,
    },
    infrastructure::fake::capturer::FakeCapturedFrame,
    infrastructure::support::media::FrameLease,
};

#[derive(kudi::DepInj)]
#[target(FakeEncoderFrameConverterImpl)]
pub(crate) struct FakeEncoderFrameConverterState;

pub(crate) struct FakeEncoderInput {
    pub(crate) capture_sequence: u64,
}

impl FakeEncoderFrameConverterState {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl<Deps> EncoderFrameConverter for FakeEncoderFrameConverterImpl<Deps>
where
    Deps: MetricsRecorder,
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;
    type EncoderInput = FakeEncoderInput;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput> {
        let convert_started_at = Instant::now();
        let input = FakeEncoderInput {
            capture_sequence: frame.capture_sequence,
        };

        self.prj_ref()
            .record_converted_frame(convert_started_at.elapsed());

        Ok(input)
    }
}
