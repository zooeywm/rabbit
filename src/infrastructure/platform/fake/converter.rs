use std::time::Instant;

use crate::{
    app::container::{
        host::outbound_port::MetricsRecorder, stream_pipeline::outbound_port::EncoderFrameConverter,
    },
    domain::stream::models::vo::FrameId,
    infrastructure::fake::capturer::FakeCapturedFrame,
    infrastructure::support::media::FrameLease,
};

#[derive(kudi::DepInj)]
#[target(FakeEncoderFrameConverterImpl)]
pub(crate) struct FakeEncoderFrameConverterState;

pub(crate) struct FakeEncoderInput {
    pub(crate) frame_id: FrameId,
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
            frame_id: frame.frame_id,
        };

        self.prj_ref()
            .record_converted_frame(input.frame_id, convert_started_at.elapsed());

        Ok(input)
    }
}
