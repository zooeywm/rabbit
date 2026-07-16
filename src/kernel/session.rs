use crate::kernel::{
    screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
    screen_manager::{Screen, ScreenId},
    session_control::ControlMessage,
    transport::{
        Delivery, Transport, TransportChannel, TransportMessage, TransportRecv,
        TransportSend,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRole {
    Controller,
    Host,
}

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
    role: SessionRole,
    send: T::SendHalf,
    recv: T::RecvHalf,
}

impl<T> Session<T>
where
    T: Transport,
{
    pub fn new(role: SessionRole, transport: T) -> Self {
        let (send, recv) = transport.split();
        Self { role, send, recv }
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn send_screen_list<'a>(
        &'a self,
        screens: &'a [Screen],
    ) -> impl Future<Output = eros::Result<()>> + 'a {
        async move {
            self.require_role(SessionRole::Host, "send a screen list")?;
            self.send_control(screens).await
        }
    }

    pub fn send_screen_streams_request(
        &self,
        request: SetScreenStreams,
    ) -> impl Future<Output = eros::Result<()>> + '_ {
        async move {
            self.require_role(SessionRole::Controller, "request screen streams")?;
            self.send_control(request).await
        }
    }

    pub fn send_screen_streams_configured(
        &self,
        configured: ScreenStreamsConfigured,
    ) -> impl Future<Output = eros::Result<()>> + '_ {
        async move {
            self.require_role(SessionRole::Host, "send screen stream results")?;
            self.send_control(configured).await
        }
    }

    pub fn recv(
        &mut self,
    ) -> impl Future<Output = eros::Result<Option<SessionMessage>>> {
        async move {
            let Some(message) = self.recv.recv().await? else {
                return Ok(None);
            };

            match message.channel {
                TransportChannel::Control => {
                    Ok(Some(SessionMessage::Control(message.try_into()?)))
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

    fn send_control<'a, M: 'a>(
        &'a self,
        message: M,
    ) -> impl Future<Output = eros::Result<()>> + 'a
    where
        TransportMessage: TryFrom<M, Error = eros::ErrorUnion>,
    {
        async move { self.send.send(message.try_into()?).await }
    }

    fn require_role(&self, expected: SessionRole, operation: &str) -> eros::Result<()> {
        if self.role != expected {
            eros::bail!(
                "Session role {:?} cannot {operation}; expected {:?}",
                self.role,
                expected
            );
        }

        Ok(())
    }
}
