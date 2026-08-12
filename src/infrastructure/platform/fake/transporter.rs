use std::time::Instant;

use eros::Context;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit, network::outbound_port::Transporter,
        root::outbound_port::NetworkMetricsRecorder,
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

#[derive(kudi::DepInj)]
#[target(FakeTransporterImpl)]
pub(crate) struct FakeTransporterState {
    packetized_unit_count: u64,
    sent_unit_count: u64,
}

pub(crate) struct FakePacketized {
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    payload: [u8; 8],
}

impl FakeTransporterState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            packetized_unit_count: 0,
            sent_unit_count: 0,
        })
    }
}

impl<Deps> Transporter for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState> + NetworkMetricsRecorder,
{
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;

    fn packetize(
        &mut self,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        let packetize_started_at = Instant::now();
        {
            let state = self.prj_ref_mut().as_mut();
            state.packetized_unit_count = state
                .packetized_unit_count
                .checked_add(1)
                .with_context(|| "Fake transporter packetized unit count space is exhausted")?;
        }

        let capture_source_id = unit.source_frame_id.capture_source_id();
        self.prj_ref().record_packetized_frame(
            capture_source_id,
            stream_id,
            unit.source_frame_id,
            packetize_started_at.elapsed(),
        );
        Ok(FakePacketized {
            capture_source_id,
            stream_id,
            payload: unit.data,
        })
    }

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()> {
        {
            let state = self.prj_ref_mut().as_mut();
            state.sent_unit_count = state
                .sent_unit_count
                .checked_add(1)
                .with_context(|| "Fake transporter sent unit count space is exhausted")?;
        }

        self.prj_ref().record_sent_bytes(
            packetized.capture_source_id,
            packetized.stream_id,
            packetized.payload.len(),
        );
        Ok(())
    }
}
