use std::time::Instant;

use crate::app::container::{
    packetization::outbound_port::Packetizer, root::outbound_port::MetricsRecorder,
    stream_pipeline::outbound_port::EncodedVideoFrame,
};

use eros::Context;

#[derive(kudi::DepInj)]
#[target(FakePacketizerImpl)]
pub(crate) struct FakePacketizerState {
    packetized_frame_count: u64,
}

impl FakePacketizerState {
    pub(crate) fn new() -> Self {
        Self {
            packetized_frame_count: 0,
        }
    }
}

impl<Deps> Packetizer for FakePacketizerImpl<Deps>
where
    Deps: AsMut<FakePacketizerState> + MetricsRecorder,
{
    type EncodedBuffer = [u8; 8];

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        let packetize_started_at = Instant::now();
        let state = self.prj_ref_mut().as_mut();
        state.packetized_frame_count = state
            .packetized_frame_count
            .checked_add(1)
            .with_context(|| "Fake packetizer frame count space is exhausted")?;

        self.prj_ref()
            .record_packetized_frame(frame.frame_id, packetize_started_at.elapsed());

        Ok(())
    }
}
