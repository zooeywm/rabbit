//! Session API: role-gated control/media over a transport.
//!
//! - [`Session`] / [`SessionSend`] / [`SessionRecv`] — public session surface
//! - [`rtp`] — Controller-side H.264 RTP reassembly (private to this package)
//! - [`role`] — authorization helpers for send/receive operations
//!
//! See `ARCHITECTURE.md` §3 for role and RTP policy.

use std::collections::HashMap;

use eros::Context as _;

use crate::kernel::{
    screen_configuration::{
        RequestKeyFrame, ScreenStreamsConfigured, SetScreenStreams, StopScreenStream,
    },
    screen_manager::ScreenId,
    session_control::{ControlMessage, OutgoingScreenList},
    transport::{
        Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
    },
};

mod role;
pub(crate) mod rtp;

use role::{require_role, validate_received_control};
use rtp::{RtpVideoStream, assemble_video_frame};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRole {
    Controller,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoMessage {
    pub screen_id: ScreenId,
    pub payload: bytes::Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedVideoFrame {
    pub screen_id: ScreenId,
    pub packets: Vec<bytes::Bytes>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionMessage {
    Control(ControlMessage),
    Video(ReceivedVideoFrame),
    KeyFrameRequired(ScreenId),
}

pub struct Session<T>
where
    T: Transport,
{
    id: SessionId,
    role: SessionRole,
    send: T::SendHalf,
    recv: T::RecvHalf,
}

pub struct SessionSend<S>
where
    S: TransportSend,
{
    id: SessionId,
    role: SessionRole,
    send: S,
}

pub struct SessionRecv<R>
where
    R: TransportRecv,
{
    id: SessionId,
    role: SessionRole,
    recv: R,
    video_streams: HashMap<ScreenId, RtpVideoStream>,
}

impl<T> Session<T>
where
    T: Transport,
{
    pub fn new(id: SessionId, role: SessionRole, transport: T) -> Self {
        let (send, recv) = transport.split();
        Self {
            id,
            role,
            send,
            recv,
        }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn split(self) -> (SessionSend<T::SendHalf>, SessionRecv<T::RecvHalf>) {
        (
            SessionSend {
                id: self.id,
                role: self.role,
                send: self.send,
            },
            SessionRecv {
                id: self.id,
                role: self.role,
                recv: self.recv,
                video_streams: HashMap::new(),
            },
        )
    }
}

impl<S> SessionSend<S>
where
    S: TransportSend,
{
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn max_video_packet_size(&self) -> Option<usize> {
        self.send.max_unreliable_payload_size()
    }

    pub fn is_closed_normally(&self) -> bool {
        self.send.is_closed_normally()
    }

    pub fn close(&self) -> impl Future<Output = ()> {
        self.send.close()
    }

    pub async fn send_video(&self, message: VideoMessage) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send video")?;
        let screen_id = message.screen_id;

        self.send
            .send_unreliable(TransportChannel::Video(screen_id), message.payload)
            .await
            .with_context(|| format!("Failed to send video packet for screen {}", screen_id.0))
    }

    pub async fn send_screen_list(&self, screens: OutgoingScreenList) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send a screen list")?;
        self.send
            .send(screens.into())
            .await
            .with_context(|| "Failed to send the Session screen list")
    }

    pub async fn send_screen_streams_request(&self, request: SetScreenStreams) -> eros::Result<()> {
        require_role(self.role, SessionRole::Controller, "request screen streams")?;
        self.send_control(request)
            .await
            .with_context(|| "Failed to send the screen stream request")
    }

    pub async fn stop_screen_stream(&self, screen_id: ScreenId) -> eros::Result<()> {
        self.send_control(StopScreenStream { screen_id })
            .await
            .with_context(|| format!("Failed to stop screen {} stream", screen_id.0))
    }

    pub async fn request_key_frame(&self, screen_id: ScreenId) -> eros::Result<()> {
        require_role(self.role, SessionRole::Controller, "request a key frame")?;
        self.send_control(RequestKeyFrame { screen_id })
            .await
            .with_context(|| format!("Failed to request a key frame for screen {}", screen_id.0))
    }

    pub async fn send_screen_streams_configured(
        &self,
        configured: ScreenStreamsConfigured,
    ) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send screen stream results")?;
        self.send_control(configured)
            .await
            .with_context(|| "Failed to send the screen stream configuration result")
    }

    async fn send_control<M>(&self, message: M) -> eros::Result<()>
    where
        TransportMessage: TryFrom<M, Error = eros::ErrorUnion>,
    {
        self.send.send(message.try_into()?).await
    }
}

impl<R> SessionRecv<R>
where
    R: TransportRecv,
{
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub async fn recv(&mut self) -> eros::Result<Option<SessionMessage>> {
        loop {
            let Some(message) = self
                .recv
                .recv()
                .await
                .with_context(|| "Failed to receive the next Session Transport message")?
            else {
                return Ok(None);
            };

            match message.channel {
                TransportChannel::Control => {
                    let message = ControlMessage::try_from(message)?;
                    validate_received_control(self.role, &message)?;

                    return Ok(Some(SessionMessage::Control(message)));
                }
                TransportChannel::Video(screen_id) => {
                    require_role(self.role, SessionRole::Controller, "receive video")?;
                    if message.delivery != Delivery::Unreliable {
                        eros::bail!(
                            "Video message for screen {} has invalid delivery {:?}",
                            screen_id.0,
                            message.delivery
                        );
                    }

                    let assembled =
                        assemble_video_frame(&mut self.video_streams, screen_id, message.payload)?;
                    if assembled.request_key_frame {
                        return Ok(Some(SessionMessage::KeyFrameRequired(screen_id)));
                    }
                    if let Some(frame) = assembled.frame {
                        return Ok(Some(SessionMessage::Video(frame)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        future::ready,
    };

    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_manager::{Screen, ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        session::{
            SessionId, SessionRecv, SessionRole, SessionSend, VideoMessage,
            rtp::assemble_video_frame,
        },
        session_control::{ControlMessage, OutgoingScreenList},
        transport::{Delivery, TransportChannel, TransportMessage, TransportRecv, TransportSend},
    };
    use bytes::Bytes;

    fn rtp_packet(sequence: u16, timestamp: u32, marker: bool) -> Bytes {
        let mut packet = vec![0_u8; 13];
        packet[0] = 2 << 6;
        packet[1] = u8::from(marker) << 7;
        packet[2..4].copy_from_slice(&sequence.to_be_bytes());
        packet[4..8].copy_from_slice(&timestamp.to_be_bytes());
        packet[12] = 5;
        Bytes::from(packet)
    }

    fn rtp_delta_packet(sequence: u16, timestamp: u32, marker: bool) -> Bytes {
        let mut packet = rtp_packet(sequence, timestamp, marker).to_vec();
        packet[12] = 1;
        Bytes::from(packet)
    }

    #[test]
    fn assembles_every_rtp_packet_before_publishing_a_video_frame() {
        let screen_id = ScreenId(3);
        let first = rtp_packet(41, 7, false);
        let last = rtp_packet(42, 7, true);
        let mut frames = HashMap::new();

        assert!(
            assemble_video_frame(&mut frames, screen_id, first.clone())
                .expect("First RTP packet should be accepted")
                .frame
                .is_none()
        );
        let frame = assemble_video_frame(&mut frames, screen_id, last.clone())
            .expect("Last RTP packet should complete the frame")
            .frame
            .expect("Complete RTP frame should be published");

        assert_eq!(frame.screen_id, screen_id);
        assert_eq!(frame.packets, vec![first, last]);
    }

    #[test]
    fn drops_the_whole_rtp_frame_after_a_sequence_gap() {
        let screen_id = ScreenId(4);
        let mut streams = HashMap::new();

        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(8, 11, false))
                .expect("First RTP packet should be accepted")
                .frame
                .is_none()
        );
        let dropped = assemble_video_frame(&mut streams, screen_id, rtp_packet(10, 11, true))
            .expect("Sequence gap should discard the completed frame");
        assert!(dropped.frame.is_none());
        assert!(dropped.request_key_frame);
        assert!(
            streams
                .get(&screen_id)
                .is_some_and(|stream| stream.frame.is_none())
        );
    }

    #[test]
    fn drops_a_frame_when_its_first_rtp_packet_is_missing() {
        let screen_id = ScreenId(5);
        let mut streams = HashMap::new();

        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(8, 11, true))
                .expect("Initial RTP frame should complete")
                .frame
                .is_some()
        );
        let dropped = assemble_video_frame(&mut streams, screen_id, rtp_delta_packet(10, 12, true))
            .expect("Frame with a missing first packet should be discarded");
        assert!(dropped.frame.is_none());
        assert!(dropped.request_key_frame);
    }

    #[test]
    fn requests_an_idr_when_the_first_received_frame_is_not_a_key_frame() {
        let screen_id = ScreenId(7);
        let mut streams = HashMap::new();

        let received = assemble_video_frame(&mut streams, screen_id, rtp_delta_packet(20, 1, true))
            .expect("Initial dependent frame should be handled");

        assert!(received.frame.is_none());
        assert!(received.request_key_frame);
    }

    #[test]
    fn waits_for_a_complete_idr_after_an_rtp_sequence_gap() {
        let screen_id = ScreenId(6);
        let mut streams = HashMap::new();

        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(1, 1, true))
                .expect("Initial IDR should be accepted")
                .frame
                .is_some()
        );
        let gap = assemble_video_frame(&mut streams, screen_id, rtp_delta_packet(3, 2, true))
            .expect("Sequence gap should be handled");
        assert!(gap.frame.is_none());
        assert!(gap.request_key_frame);
        let dependent = assemble_video_frame(&mut streams, screen_id, rtp_delta_packet(4, 3, true))
            .expect("Dependent frame should be discarded while waiting for IDR");
        assert!(dependent.frame.is_none());
        assert!(!dependent.request_key_frame);
        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(5, 4, true))
                .expect("Complete IDR should restore the stream")
                .frame
                .is_some()
        );
    }

    struct TestTransportSend {
        messages: RefCell<Vec<TransportMessage>>,
        closed: Cell<bool>,
    }

    struct TestTransportRecv(Option<TransportMessage>);

    impl TransportRecv for TestTransportRecv {
        fn recv(&mut self) -> impl Future<Output = eros::Result<Option<TransportMessage>>> {
            ready(Ok(self.0.take()))
        }
    }

    #[test]
    fn host_rejects_video_from_the_controller() {
        let mut session = SessionRecv {
            id: SessionId(1),
            role: SessionRole::Host,
            recv: TestTransportRecv(Some(TransportMessage {
                channel: TransportChannel::Video(ScreenId(0)),
                delivery: Delivery::Unreliable,
                payload: rtp_packet(1, 1, true),
            })),
            video_streams: HashMap::new(),
        };
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(session.recv())
            .expect_err("Host must reject video sent by its Controller peer");
    }

    impl TransportSend for TestTransportSend {
        fn max_unreliable_payload_size(&self) -> Option<usize> {
            Some(1173)
        }

        fn send_unreliable(
            &self,
            channel: TransportChannel,
            payload: Bytes,
        ) -> impl Future<Output = eros::Result<()>> {
            self.messages.borrow_mut().push(TransportMessage {
                channel,
                delivery: Delivery::Unreliable,
                payload,
            });
            ready(Ok(()))
        }

        fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>> {
            self.messages.borrow_mut().push(message);
            ready(Ok(()))
        }

        fn close(&self) -> impl Future<Output = ()> {
            self.closed.set(true);
            ready(())
        }
    }

    #[test]
    fn session_close_closes_its_transport() {
        let session = SessionSend {
            id: SessionId(6),
            role: SessionRole::Controller,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
                closed: Cell::new(false),
            },
        };
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(session.close());

        assert!(session.send.closed.get());
    }

    #[test]
    fn both_session_roles_can_stop_only_the_selected_screen() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        for role in [SessionRole::Controller, SessionRole::Host] {
            let session = SessionSend {
                id: SessionId(9),
                role,
                send: TestTransportSend {
                    messages: RefCell::new(Vec::new()),
                    closed: Cell::new(false),
                },
            };

            runtime
                .block_on(session.stop_screen_stream(ScreenId(4)))
                .expect("Selected screen stop should be sent");

            let message = session
                .send
                .messages
                .into_inner()
                .pop()
                .expect("Transport should receive the screen stop message");
            let ControlMessage::StopScreenStream(stop) =
                ControlMessage::try_from(message).expect("Screen stop should decode")
            else {
                panic!("Decoded control message should stop one screen");
            };

            assert_eq!(stop.screen_id, ScreenId(4));
        }
    }

    #[test]
    fn controller_requests_a_key_frame_for_only_the_selected_screen() {
        let session = SessionSend {
            id: SessionId(10),
            role: SessionRole::Controller,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
                closed: Cell::new(false),
            },
        };
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(session.request_key_frame(ScreenId(5)))
            .expect("Controller should request a key frame");

        let message = session
            .send
            .messages
            .into_inner()
            .pop()
            .expect("Transport should receive the key-frame request");
        let ControlMessage::RequestKeyFrame(request) =
            ControlMessage::try_from(message).expect("Key-frame request should decode")
        else {
            panic!("Decoded control message should request one key frame");
        };

        assert_eq!(request.screen_id, ScreenId(5));
    }

    #[test]
    fn host_sends_one_video_packet_through_the_screen_channel() {
        let session = SessionSend {
            id: SessionId(7),
            role: SessionRole::Host,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
                closed: Cell::new(false),
            },
        };
        let packet = Bytes::from_static(b"standard RTP packet");
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");
        runtime
            .block_on(session.send_video(VideoMessage {
                screen_id: ScreenId(3),
                payload: packet,
            }))
            .expect("Host should send one video packet");

        assert_eq!(session.max_video_packet_size(), Some(1173));
        assert_eq!(
            session.send.messages.borrow().as_slice(),
            &[TransportMessage {
                channel: TransportChannel::Video(ScreenId(3)),
                delivery: Delivery::Unreliable,
                payload: Bytes::from_static(b"standard RTP packet"),
            }]
        );
    }

    #[test]
    fn host_sends_an_owned_screen_list() {
        let session = SessionSend {
            id: SessionId(8),
            role: SessionRole::Host,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
                closed: Cell::new(false),
            },
        };
        let screens = [Screen {
            id: ScreenId(2),
            name: "eDP-1".to_owned(),
            resolution: PixelSize {
                width: 2560,
                height: 1600,
            },
            frame_rate: FrameRate::new(120_000, 1_000)
                .expect("Test screen frame rate should be valid"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 800,
                },
                scale: 2.0,
                transform: ScreenTransform::Normal,
            },
        }];
        let screen_list = OutgoingScreenList::try_from(screens.as_slice())
            .expect("Screen list should encode before it is sent");
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(session.send_screen_list(screen_list))
            .expect("Host should send the encoded screen list");

        let message = session
            .send
            .messages
            .into_inner()
            .pop()
            .expect("Transport should receive the screen list");
        let ControlMessage::ScreenList(screens) =
            ControlMessage::try_from(message).expect("Screen list should decode")
        else {
            panic!("Decoded control message should be a screen list");
        };

        assert_eq!(screens.len(), 1);
        assert_eq!(screens[0].id, ScreenId(2));
        assert_eq!(screens[0].name, "eDP-1");
        assert_eq!(screens[0].frame_rate.numerator(), 120_000);
        assert_eq!(screens[0].frame_rate.denominator(), 1_000);
    }
}
