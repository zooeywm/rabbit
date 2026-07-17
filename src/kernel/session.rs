use eros::Context as _;

use crate::kernel::{
    screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
    screen_manager::{Screen, ScreenId},
    session_control::ControlMessage,
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

#[derive(Debug, Clone, PartialEq)]
pub enum SessionMessage {
    Control(ControlMessage),
    Video(VideoMessage),
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

        Ok(self
            .send
            .send(TransportMessage {
                channel: TransportChannel::Video(screen_id),
                delivery: Delivery::Unreliable,
                payload: message.payload,
            })
            .await
            .with_context(|| format!("Failed to send video packet for screen {}", screen_id.0))?)
    }

    pub async fn send_screen_list(&self, screens: &[Screen]) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send a screen list")?;
        self.send_control(screens).await
    }

    pub async fn send_screen_streams_request(&self, request: SetScreenStreams) -> eros::Result<()> {
        require_role(self.role, SessionRole::Controller, "request screen streams")?;
        self.send_control(request).await
    }

    pub async fn send_screen_streams_configured(
        &self,
        configured: ScreenStreamsConfigured,
    ) -> eros::Result<()> {
        require_role(self.role, SessionRole::Host, "send screen stream results")?;
        self.send_control(configured).await
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
        let Some(message) = self.recv.recv().await? else {
            return Ok(None);
        };

        match message.channel {
            TransportChannel::Control => {
                let message = ControlMessage::try_from(message)?;
                validate_received_control(self.role, &message)?;

                Ok(Some(SessionMessage::Control(message)))
            }
            TransportChannel::Video(screen_id) => {
                if message.delivery != Delivery::Unreliable {
                    eros::bail!(
                        "Video message for screen {} has invalid delivery {:?}",
                        screen_id.0,
                        message.delivery
                    );
                }

                Ok(Some(SessionMessage::Video(VideoMessage {
                    screen_id,
                    payload: message.payload,
                })))
            }
        }
    }
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
    use std::{cell::RefCell, future::ready};

    use bytes::Bytes;
    use eros::Context;

    use crate::kernel::{
        screen_manager::ScreenId,
        session::{SessionId, SessionRole, SessionSend, VideoMessage},
        transport::{Delivery, TransportChannel, TransportMessage, TransportSend},
    };

    struct TestTransportSend {
        messages: RefCell<Vec<TransportMessage>>,
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
    fn host_sends_one_video_packet_through_the_screen_channel() -> eros::Result<()> {
        let session = SessionSend {
            id: SessionId(7),
            role: SessionRole::Host,
            send: TestTransportSend {
                messages: RefCell::new(Vec::new()),
            },
        };
        let packet = Bytes::from_static(b"standard RTP packet");
        let runtime = compio::runtime::Runtime::new()
            .with_context(|| "Failed to start the Compio test runtime")?;

        runtime.block_on(session.send_video(VideoMessage {
            screen_id: ScreenId(3),
            payload: packet,
        }))?;

        assert_eq!(session.max_video_packet_size(), Some(1173));
        assert_eq!(
            session.send.messages.borrow().as_slice(),
            &[TransportMessage {
                channel: TransportChannel::Video(ScreenId(3)),
                delivery: Delivery::Unreliable,
                payload: Bytes::from_static(b"standard RTP packet"),
            }]
        );

        Ok(())
    }
}
