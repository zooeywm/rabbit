use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use eros::Context;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{SentBytes, TransporterClientSide, TransporterHostSide},
    },
    domain::stream::models::vo::{FrameId, StreamId},
    infrastructure::platform::FakeDecoderInput,
};

#[derive(kudi::DepInj)]
#[target(FakeTransporterImpl)]
pub(crate) struct FakeTransporterState {
    host: Option<FakeTransporterHost>,
    receiver: Option<FakeTransporterReceiver>,
    streams_awaiting_video_refresh: HashSet<StreamId>,
}

pub(crate) struct FakePacketized {
    frame_id: FrameId,
    stream_id: StreamId,
    is_keyframe: bool,
    payload: [u8; 8],
}

pub(crate) struct FakeTransporterHost {
    wire: Arc<Mutex<Option<FakeReceived>>>,
    notification_sender: flume::Sender<()>,
}

pub(crate) struct FakeTransporterReceiver {
    wire: Arc<Mutex<Option<FakeReceived>>>,
    notification_receiver: flume::Receiver<()>,
}

pub(crate) struct FakeReceived {
    frame_id: FrameId,
    stream_id: StreamId,
    is_keyframe: bool,
    payload: [u8; 8],
}

impl FakeTransporterHost {
    pub(crate) fn packetize(
        &mut self,
        stream_id: StreamId,
        unit: EncodedVideoUnit<[u8; 8]>,
    ) -> FakePacketized {
        FakePacketized {
            frame_id: unit.source_frame_id,
            stream_id,
            is_keyframe: unit.is_keyframe,
            payload: unit.data,
        }
    }

    pub(crate) async fn send(&mut self, packetized: FakePacketized) -> eros::Result<SentBytes> {
        let received = FakeReceived {
            frame_id: packetized.frame_id,
            stream_id: packetized.stream_id,
            is_keyframe: packetized.is_keyframe,
            payload: packetized.payload,
        };
        *self
            .wire
            .lock()
            .expect("fake wire mutex should not be poisoned") = Some(received);
        match self.notification_sender.try_send(()) {
            Ok(()) | Err(flume::TrySendError::Full(())) => {}
            Err(flume::TrySendError::Disconnected(())) => {
                eros::bail!("Fake transporter receiver stopped before send completed");
            }
        }

        Ok(SentBytes::new(
            packetized.frame_id.capture_source_id(),
            packetized.stream_id,
            packetized.payload.len(),
        ))
    }
}

impl FakeTransporterReceiver {
    pub(crate) async fn receive(&mut self) -> eros::Result<Option<FakeReceived>> {
        loop {
            self.notification_receiver.recv_async().await.with_context(
                || "Fake transporter sender stopped while the connection was active",
            )?;
            if let Some(received) = self
                .wire
                .lock()
                .expect("fake wire mutex should not be poisoned")
                .take()
            {
                return Ok(Some(received));
            }
        }
    }
}

impl FakeTransporterState {
    pub(crate) fn new() -> eros::Result<Self> {
        let wire = Arc::new(Mutex::new(None));
        let (notification_sender, notification_receiver) = flume::bounded(1);
        Ok(Self {
            host: Some(FakeTransporterHost {
                wire: Arc::clone(&wire),
                notification_sender,
            }),
            receiver: Some(FakeTransporterReceiver {
                wire,
                notification_receiver,
            }),
            streams_awaiting_video_refresh: HashSet::new(),
        })
    }
}

impl<Deps> TransporterHostSide for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState>,
{
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;
    type Host = FakeTransporterHost;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .host
            .take()
            .with_context(|| "Fake transporter host half has already been taken")?)
    }

    fn packetize(
        host: &mut Self::Host,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        Ok(host.packetize(stream_id, unit))
    }

    async fn send(host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes> {
        host.send(packetized).await
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
            .host
            .as_ref()
            .expect("fake host half should exist")
            .notification_sender
            .capacity();

        assert_eq!(capacity, Some(1));
        Ok(())
    }

    #[test]
    fn full_fake_wire_keeps_only_the_latest_unit() -> eros::Result<()> {
        let mut state = FakeTransporterState::new()?;
        let mut host = state.host.take().expect("fake host half should exist");
        let mut receiver = state.receiver.take().expect("fake receiver should exist");
        let runtime = compio::runtime::Runtime::new()?;

        runtime.block_on(async {
            for sequence in 0..=1 {
                host.send(FakePacketized {
                    frame_id: FrameId::new(CaptureSourceId::new(0), sequence),
                    stream_id: StreamId::new(0),
                    is_keyframe: false,
                    payload: sequence.to_le_bytes(),
                })
                .await?;
            }

            let received = receiver
                .receive()
                .await?
                .with_context(|| "fake wire stopped before returning its latest unit")?;
            assert!(received.payload == 1_u64.to_le_bytes());
            eros::Result::Ok(())
        })
    }

    #[test]
    fn fake_send_propagates_a_closed_wire() -> eros::Result<()> {
        let mut state = FakeTransporterState::new()?;
        let mut host = state.host.take().expect("fake host half should exist");
        drop(state.receiver.take());
        let runtime = compio::runtime::Runtime::new()?;
        let result = runtime.block_on(host.send(FakePacketized {
            frame_id: FrameId::new(CaptureSourceId::new(0), 0),
            stream_id: StreamId::new(0),
            is_keyframe: true,
            payload: [0; 8],
        }));

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn fake_receive_propagates_an_unexpected_closed_wire() -> eros::Result<()> {
        let mut state = FakeTransporterState::new()?;
        drop(state.host.take());
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
        let state = self.prj_ref_mut().as_mut();
        let recovery_point = received.is_keyframe
            || state
                .streams_awaiting_video_refresh
                .remove(&received.stream_id);
        Ok((
            received.stream_id,
            FakeDecoderInput::new(received.frame_id, received.payload, recovery_point),
        ))
    }

    fn request_video_refresh(&mut self, stream_id: StreamId) -> eros::Result<()> {
        self.prj_ref_mut()
            .as_mut()
            .streams_awaiting_video_refresh
            .insert(stream_id);
        Ok(())
    }
}
