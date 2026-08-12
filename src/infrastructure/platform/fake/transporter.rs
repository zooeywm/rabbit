use std::time::Instant;

use eros::Context;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{
            NetworkMetricsRecorder, SentBytes, TransporterClientSide, TransporterHostSide,
        },
    },
    domain::stream::models::vo::{FrameId, StreamId},
    infrastructure::platform::FakeDecoderInput,
};

#[derive(kudi::DepInj)]
#[target(FakeTransporterImpl)]
pub(crate) struct FakeTransporterState {
    packetized_unit_count: u64,
    sender: Option<FakeTransporterSender>,
    receiver: Option<FakeTransporterReceiver>,
}

pub(crate) struct FakePacketized {
    frame_id: FrameId,
    stream_id: StreamId,
    payload: [u8; 8],
}

pub(crate) struct FakeTransporterSender {
    sent_unit_count: u64,
    sender: flume::Sender<FakeReceived>,
}

pub(crate) struct FakeTransporterReceiver {
    receiver: flume::Receiver<FakeReceived>,
}

pub(crate) struct FakeReceived {
    frame_id: FrameId,
    stream_id: StreamId,
    payload: [u8; 8],
}

impl FakeTransporterSender {
    pub(crate) async fn send(&mut self, packetized: FakePacketized) -> eros::Result<SentBytes> {
        let sent_unit_count = self
            .sent_unit_count
            .checked_add(1)
            .with_context(|| "Fake transporter sent unit count space is exhausted")?;

        self.sender
            .send_async(FakeReceived {
                frame_id: packetized.frame_id,
                stream_id: packetized.stream_id,
                payload: packetized.payload,
            })
            .await
            .with_context(|| "Fake transporter receiver stopped before send completed")?;

        self.sent_unit_count = sent_unit_count;

        Ok(SentBytes::new(
            packetized.frame_id.capture_source_id(),
            packetized.stream_id,
            packetized.payload.len(),
        ))
    }
}

impl FakeTransporterReceiver {
    pub(crate) async fn receive(&mut self) -> eros::Result<Option<FakeReceived>> {
        Ok(self.receiver.recv_async().await.ok())
    }
}

impl FakeTransporterState {
    pub(crate) fn new() -> eros::Result<Self> {
        let (sender, receiver) = flume::bounded(1);
        Ok(Self {
            packetized_unit_count: 0,
            sender: Some(FakeTransporterSender {
                sent_unit_count: 0,
                sender,
            }),
            receiver: Some(FakeTransporterReceiver { receiver }),
        })
    }
}

impl<Deps> TransporterHostSide for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState> + NetworkMetricsRecorder,
{
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;
    type Sender = FakeTransporterSender;

    fn take_sender(&mut self) -> eros::Result<Self::Sender> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .sender
            .take()
            .with_context(|| "Fake transporter sender has already been taken")?)
    }

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
            frame_id: unit.source_frame_id,
            stream_id,
            payload: unit.data,
        })
    }

    async fn send(
        sender: &mut Self::Sender,
        packetized: Self::Packetized,
    ) -> eros::Result<SentBytes> {
        sender.send(packetized).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::stream::models::vo::CaptureSourceId;

    #[test]
    fn fake_wire_is_bounded_to_one_unit() -> eros::Result<()> {
        let state = FakeTransporterState::new()?;
        let capacity = state
            .sender
            .as_ref()
            .expect("fake sender should exist")
            .sender
            .capacity();

        assert_eq!(capacity, Some(1));
        Ok(())
    }

    #[test]
    fn fake_send_propagates_a_closed_wire() -> eros::Result<()> {
        let mut state = FakeTransporterState::new()?;
        let mut sender = state.sender.take().expect("fake sender should exist");
        drop(state.receiver.take());
        let runtime = compio::runtime::Runtime::new()?;
        let result = runtime.block_on(sender.send(FakePacketized {
            frame_id: FrameId::new(CaptureSourceId::new(0), 0),
            stream_id: StreamId::new(0),
            payload: [0; 8],
        }));

        assert!(result.is_err());
        assert_eq!(sender.sent_unit_count, 0);
        Ok(())
    }
}

impl<Deps> TransporterClientSide for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState>,
{
    type Receiver = FakeTransporterReceiver;
    type Received = FakeReceived;
    type Depacketized = FakeDecoderInput;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .receiver
            .take()
            .with_context(|| "Fake transporter receiver has already been taken")?)
    }

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        receiver.receive().await
    }

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(StreamId, Self::Depacketized)> {
        Ok((
            received.stream_id,
            FakeDecoderInput::new(received.frame_id, received.payload),
        ))
    }
}
