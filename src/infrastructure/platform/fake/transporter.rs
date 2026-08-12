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
    sender: Option<FakeTransporterSender>,
    receiver: Option<FakeTransporterReceiver>,
}

pub(crate) struct FakePacketized {
    frame_id: FrameId,
    stream_id: StreamId,
    payload: [u8; 8],
}

pub(crate) struct FakeTransporterSender {
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
        self.sender
            .send_async(FakeReceived {
                frame_id: packetized.frame_id,
                stream_id: packetized.stream_id,
                payload: packetized.payload,
            })
            .await
            .with_context(|| "Fake transporter receiver stopped before send completed")?;

        Ok(SentBytes::new(
            packetized.frame_id.capture_source_id(),
            packetized.stream_id,
            packetized.payload.len(),
        ))
    }
}

impl FakeTransporterReceiver {
    pub(crate) async fn receive(&mut self) -> eros::Result<Option<FakeReceived>> {
        Ok(Some(self.receiver.recv_async().await.with_context(
            || "Fake transporter sender stopped while the connection was active",
        )?))
    }
}

impl FakeTransporterState {
    pub(crate) fn new() -> eros::Result<Self> {
        let (sender, receiver) = flume::bounded(1);
        Ok(Self {
            sender: Some(FakeTransporterSender { sender }),
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
        Ok(())
    }

    #[test]
    fn fake_receive_propagates_an_unexpected_closed_wire() -> eros::Result<()> {
        let mut state = FakeTransporterState::new()?;
        drop(state.sender.take());
        let mut receiver = state.receiver.take().expect("fake receiver should exist");
        let runtime = compio::runtime::Runtime::new()?;
        let error = match runtime.block_on(receiver.receive()) {
            Ok(_) => panic!("closed fake wire should fail while active"),
            Err(error) => error,
        };

        assert!(
            format!("{error:?}")
                .contains("Fake transporter sender stopped while the connection was active")
        );
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
