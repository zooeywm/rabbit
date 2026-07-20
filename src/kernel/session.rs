use std::collections::HashMap;

use eros::Context as _;

use crate::kernel::{
    screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
    screen_manager::ScreenId,
    session_control::{ControlMessage, OutgoingScreenList},
    transport::{
        Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
    },
};

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

#[derive(Default)]
struct RtpVideoStream {
    next_sequence: Option<u16>,
    frame: Option<RtpFrameAssembly>,
}

struct RtpFrameAssembly {
    timestamp: u32,
    packets: Vec<bytes::Bytes>,
    payload_size: usize,
    valid: bool,
}

struct RtpPacketMetadata {
    sequence: u16,
    timestamp: u32,
    marker: bool,
}

const RTP_FIXED_HEADER_SIZE: usize = 12;
const MAX_ENCODED_VIDEO_FRAME_SIZE: usize = 16 * 1024 * 1024;

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

    pub async fn send_video(&self, message: VideoMessage) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send video")?;
        let screen_id = message.screen_id;

        self.send
            .send(TransportMessage {
                channel: TransportChannel::Video(screen_id),
                delivery: Delivery::Unreliable,
                payload: message.payload,
            })
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

                    if let Some(frame) =
                        assemble_video_frame(&mut self.video_streams, screen_id, message.payload)?
                    {
                        return Ok(Some(SessionMessage::Video(frame)));
                    }
                }
            }
        }
    }
}

fn assemble_video_frame(
    streams: &mut HashMap<ScreenId, RtpVideoStream>,
    screen_id: ScreenId,
    packet: bytes::Bytes,
) -> eros::Result<Option<ReceivedVideoFrame>> {
    let metadata = decode_rtp_metadata(&packet)?;
    let packet_size = packet.len();
    let stream = streams.entry(screen_id).or_default();
    let sequence_is_contiguous = stream
        .next_sequence
        .is_none_or(|expected| metadata.sequence == expected);
    stream.next_sequence = Some(metadata.sequence.wrapping_add(1));

    if stream
        .frame
        .as_ref()
        .is_none_or(|frame| frame.timestamp != metadata.timestamp)
    {
        stream.frame = Some(RtpFrameAssembly {
            timestamp: metadata.timestamp,
            packets: Vec::new(),
            payload_size: 0,
            valid: sequence_is_contiguous,
        });
    }
    let frame = stream
        .frame
        .as_mut()
        .with_context(|| format!("RTP frame for screen {} is missing", screen_id.0))?;

    if !sequence_is_contiguous {
        frame.valid = false;
    }
    frame.payload_size = frame
        .payload_size
        .checked_add(packet_size)
        .with_context(|| format!("RTP frame size overflow for screen {}", screen_id.0))?;
    if frame.payload_size > MAX_ENCODED_VIDEO_FRAME_SIZE {
        eros::bail!(
            "RTP frame for screen {} exceeds {} bytes",
            screen_id.0,
            MAX_ENCODED_VIDEO_FRAME_SIZE
        );
    }
    frame.packets.push(packet);

    if !metadata.marker {
        return Ok(None);
    }

    let frame = stream
        .frame
        .take()
        .with_context(|| format!("Completed RTP frame for screen {} is missing", screen_id.0))?;
    if !frame.valid {
        return Ok(None);
    }

    Ok(Some(ReceivedVideoFrame {
        screen_id,
        packets: frame.packets,
    }))
}

fn decode_rtp_metadata(packet: &bytes::Bytes) -> eros::Result<RtpPacketMetadata> {
    if packet.len() < RTP_FIXED_HEADER_SIZE {
        eros::bail!(
            "Video RTP packet is {} bytes, shorter than the fixed {}-byte header",
            packet.len(),
            RTP_FIXED_HEADER_SIZE
        );
    }
    let version = packet[0] >> 6;
    if version != 2 {
        eros::bail!("Video RTP packet has unsupported version {version}");
    }

    Ok(RtpPacketMetadata {
        sequence: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        marker: packet[1] & 0x80 != 0,
    })
}

fn require_role(role: SessionRole, expected: SessionRole, operation: &str) -> eros::Result<()> {
    if role != expected {
        eros::bail!(
            "Session role {:?} cannot {operation}; expected {:?}",
            role,
            expected
        );
    }

    Ok(())
}

fn validate_received_control(role: SessionRole, message: &ControlMessage) -> eros::Result<()> {
    let (expected, name) = match message {
        ControlMessage::ScreenList(_) => (SessionRole::Controller, "ScreenList"),
        ControlMessage::SetScreenStreams(_) => (SessionRole::Host, "SetScreenStreams"),
        ControlMessage::ScreenStreamsConfigured(_) => {
            (SessionRole::Controller, "ScreenStreamsConfigured")
        }
    };

    if role != expected {
        eros::bail!(
            "Session role {:?} cannot receive {name}; expected {:?}",
            role,
            expected
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, future::ready};

    use crate::kernel::{
        geometry::PixelSize,
        screen_manager::{Screen, ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        session::{
            SessionId, SessionRecv, SessionRole, SessionSend, VideoMessage, assemble_video_frame,
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
        packet[12] = sequence as u8;
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
                .is_none()
        );
        let frame = assemble_video_frame(&mut frames, screen_id, last.clone())
            .expect("Last RTP packet should complete the frame")
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
                .is_none()
        );
        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(10, 11, true))
                .expect("Sequence gap should discard the completed frame")
                .is_none()
        );
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
                .is_some()
        );
        assert!(
            assemble_video_frame(&mut streams, screen_id, rtp_packet(10, 12, true))
                .expect("Frame with a missing first packet should be discarded")
                .is_none()
        );
    }

    struct TestTransportSend {
        messages: RefCell<Vec<TransportMessage>>,
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

        fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>> {
            self.messages.borrow_mut().push(message);
            ready(Ok(()))
        }
    }

    #[test]
    fn host_sends_one_video_packet_through_the_screen_channel() {
        let session = SessionSend {
            id: SessionId(7),
            role: SessionRole::Host,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
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
            },
        };
        let screens = [Screen {
            id: ScreenId(2),
            name: "eDP-1".to_owned(),
            resolution: PixelSize {
                width: 2560,
                height: 1600,
            },
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
    }
}
