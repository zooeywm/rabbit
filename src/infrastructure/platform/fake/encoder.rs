use std::time::Instant;

use eros::Context;

use crate::{
    app::container::{
        host::outbound_port::MetricsRecorder,
        stream_pipeline::outbound_port::{EncodedVideoUnit, UnitNumber, VideoEncoder},
    },
    infrastructure::fake::converter::FakeEncoderInput,
};

#[derive(kudi::DepInj)]
#[target(FakeVideoEncoderImpl)]
pub(crate) struct FakeVideoEncoderState {
    next_unit_number: u64,
}

impl FakeVideoEncoderState {
    pub(crate) fn new() -> Self {
        Self {
            next_unit_number: 0,
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
    ) -> eros::Result<EncodedVideoUnit<Self::EncodedBuffer>> {
        let encode_started_at = Instant::now();
        let state = self.prj_ref_mut().as_mut();
        let unit_number = state.next_unit_number;

        state.next_unit_number = state
            .next_unit_number
            .checked_add(1)
            .with_context(|| "Fake video encoder unit number space is exhausted")?;

        let unit = EncodedVideoUnit::new(
            input.frame_id,
            UnitNumber::new(unit_number),
            unit_number == 0,
            unit_number.to_le_bytes(),
        );

        self.prj_ref()
            .record_encoded_frame(input.frame_id, encode_started_at.elapsed());

        Ok(unit)
    }
}
