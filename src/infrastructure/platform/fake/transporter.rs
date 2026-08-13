use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use eros::Context;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{
            NetworkState, SentBytes, TransporterClientSide, TransporterHostSide,
        },
    },
    domain::stream::models::{
        StreamRequest,
        vo::{FrameId, StreamId},
    },
    infrastructure::platform::FakeDecoderInput,
};

#[derive(kudi::DepInj)]
#[target(FakeTransporterImpl)]
pub(crate) struct FakeTransporterState {
    host: Option<FakeTransporterHost>,
    receiver: Option<FakeTransporterReceiver>,
    request_sender: Option<FakeTransporterRequestSender>,
    request_receiver: Option<FakeTransporterRequestReceiver>,
    streams_awaiting_video_refresh: HashSet<StreamId>,
}

pub(crate) struct FakeTransporterConfig {
    host: FakeTransporterHost,
    receiver: FakeTransporterReceiver,
    request_sender: FakeTransporterRequestSender,
    request_receiver: FakeTransporterRequestReceiver,
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

pub(crate) struct FakeTransporterRequestSender {
    sender: flume::Sender<StreamRequest>,
}

pub(crate) struct FakeTransporterRequestReceiver {
    receiver: flume::Receiver<StreamRequest>,
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

impl FakeTransporterRequestSender {
    pub(crate) async fn send(&mut self, request: StreamRequest) -> eros::Result<()> {
        self.sender
            .send_async(request)
            .await
            .map_err(|_| eros::error!("Fake remote stopped before receiving network request"))
    }
}

impl FakeTransporterRequestReceiver {
    pub(crate) async fn receive(&mut self) -> eros::Result<Option<StreamRequest>> {
        Ok(self.receiver.recv_async().await.ok())
    }
}

impl FakeTransporterState {
    pub(crate) fn new(config: FakeTransporterConfig) -> Self {
        Self {
            host: Some(config.host),
            receiver: Some(config.receiver),
            request_sender: Some(config.request_sender),
            request_receiver: Some(config.request_receiver),
            streams_awaiting_video_refresh: HashSet::new(),
        }
    }
}

impl FakeTransporterConfig {
    pub(crate) fn pair() -> (Self, Self) {
        let (a_to_b_host, b_receiver) = fake_media_direction();
        let (b_to_a_host, a_receiver) = fake_media_direction();
        let (a_request_sender, b_request_receiver) = fake_request_direction();
        let (b_request_sender, a_request_receiver) = fake_request_direction();

        (
            Self {
                host: a_to_b_host,
                receiver: a_receiver,
                request_sender: a_request_sender,
                request_receiver: a_request_receiver,
            },
            Self {
                host: b_to_a_host,
                receiver: b_receiver,
                request_sender: b_request_sender,
                request_receiver: b_request_receiver,
            },
        )
    }
}

fn fake_media_direction() -> (FakeTransporterHost, FakeTransporterReceiver) {
    let wire = Arc::new(Mutex::new(None));
    let (notification_sender, notification_receiver) = flume::bounded(1);
    (
        FakeTransporterHost {
            wire: Arc::clone(&wire),
            notification_sender,
        },
        FakeTransporterReceiver {
            wire,
            notification_receiver,
        },
    )
}

fn fake_request_direction() -> (FakeTransporterRequestSender, FakeTransporterRequestReceiver) {
    let (sender, receiver) = flume::bounded(32);
    (
        FakeTransporterRequestSender { sender },
        FakeTransporterRequestReceiver { receiver },
    )
}

impl NetworkState for FakeTransporterState {
    type Config = FakeTransporterConfig;
    type EncodedBuffer = [u8; 8];
    type ClientInput = FakeDecoderInput;

    fn new(config: Self::Config) -> eros::Result<Self> {
        Ok(Self::new(config))
    }
}

impl<Deps> TransporterHostSide for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState>,
{
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;
    type Host = FakeTransporterHost;
    type RequestReceiver = FakeTransporterRequestReceiver;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .host
            .take()
            .with_context(|| "Fake transporter host half has already been taken")?)
    }

    fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .request_receiver
            .take()
            .with_context(|| "Fake transporter request receiver has already been taken")?)
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

    async fn receive_request(
        receiver: &mut Self::RequestReceiver,
    ) -> eros::Result<Option<StreamRequest>> {
        receiver.receive().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::stream::models::vo::CaptureSourceId;

    #[test]
    fn fake_wire_is_bounded_to_one_unit() -> eros::Result<()> {
        let (config, _peer_config) = FakeTransporterConfig::pair();
        let state = FakeTransporterState::new(config);
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
        let (a_config, b_config) = FakeTransporterConfig::pair();
        let mut a = FakeTransporterState::new(a_config);
        let mut b = FakeTransporterState::new(b_config);
        let mut host = a.host.take().expect("fake A host half should exist");
        let mut receiver = b.receiver.take().expect("fake B receiver should exist");
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
        let (a_config, b_config) = FakeTransporterConfig::pair();
        let mut a = FakeTransporterState::new(a_config);
        let mut b = FakeTransporterState::new(b_config);
        let mut host = a.host.take().expect("fake A host half should exist");
        drop(b.receiver.take());
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
        let (a_config, b_config) = FakeTransporterConfig::pair();
        let mut a = FakeTransporterState::new(a_config);
        let mut b = FakeTransporterState::new(b_config);
        drop(a.host.take());
        let mut receiver = b.receiver.take().expect("fake B receiver should exist");
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

    #[test]
    fn fake_request_is_delivered_only_to_peer() -> eros::Result<()> {
        let (a_config, b_config) = FakeTransporterConfig::pair();
        let mut a = FakeTransporterState::new(a_config);
        let mut b = FakeTransporterState::new(b_config);
        let mut sender = a
            .request_sender
            .take()
            .expect("fake A request sender should exist");
        let mut peer_receiver = b
            .request_receiver
            .take()
            .expect("fake B request receiver should exist");
        let local_receiver = a
            .request_receiver
            .as_ref()
            .expect("fake A request receiver should exist");
        let stream_id = StreamId::new(7);
        let runtime = compio::runtime::Runtime::new()?;

        runtime.block_on(async {
            sender.send(StreamRequest::Remove { stream_id }).await?;
            assert!(local_receiver.receiver.is_empty());
            let Some(StreamRequest::Remove {
                stream_id: received_stream_id,
            }) = peer_receiver.receive().await?
            else {
                eros::bail!("Fake peer did not receive Remove stream request");
            };
            assert!(received_stream_id == stream_id);
            eros::Result::Ok(())
        })
    }
}

impl<Deps> TransporterClientSide for FakeTransporterImpl<Deps>
where
    Deps: AsMut<FakeTransporterState>,
{
    type Receiver = FakeTransporterReceiver;
    type RequestSender = FakeTransporterRequestSender;
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

    fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender> {
        Ok(self
            .prj_ref_mut()
            .as_mut()
            .request_sender
            .take()
            .with_context(|| "Fake transporter request sender has already been taken")?)
    }

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        receiver.receive().await
    }

    async fn send_request(
        sender: &mut Self::RequestSender,
        request: StreamRequest,
    ) -> eros::Result<()> {
        sender.send(request).await
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
