use crate::app::container::{
    packetization::outbound_port::Packetizer, stream_pipeline::outbound_port::EncodedVideoFrame,
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
    Deps: AsMut<FakePacketizerState>,
{
    type EncodedBuffer = [u8; 8];

    fn packetize(&mut self, _frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        let state = self.prj_ref_mut().as_mut();
        state.packetized_frame_count = state
            .packetized_frame_count
            .checked_add(1)
            .with_context(|| "Fake packetizer frame count space is exhausted")?;

        Ok(())
    }
}
