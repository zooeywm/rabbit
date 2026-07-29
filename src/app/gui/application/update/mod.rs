//! Message dispatch for the root application loop.

mod connection;
mod host_stream;
mod lifecycle;
mod remote_video;
mod session;

use crate::app::{
    gui::application::{
        RootApplication,
        message::{MessageSender, RootMessage},
    },
    platform::ApplicationStack,
};

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) async fn update(
        &mut self,
        message: RootMessage,
        sender: &MessageSender,
    ) -> eros::Result<bool> {
        if self.lifecycle.closing && !matches!(&message, RootMessage::ShutdownFinished) {
            return Ok(false);
        }

        match message {
            message @ (RootMessage::SelectSection(..)
            | RootMessage::Close
            | RootMessage::ShutdownFinished) => self.handle_lifecycle(message, sender).await,
            message @ (RootMessage::ResetDirectConnection
            | RootMessage::ConnectDirect(..)
            | RootMessage::DirectConnectionFinished(..)
            | RootMessage::ConnectionRequest(..)
            | RootMessage::AcceptConnectionRequest(..)
            | RootMessage::RejectConnectionRequest(..)
            | RootMessage::ConnectionAccepted { .. }
            | RootMessage::InitialScreenListFinished { .. }
            | RootMessage::ConnectionRejected(..)
            | RootMessage::ConnectionRequestFailed(..)
            | RootMessage::ConnectionListenerFailed(..)) => {
                self.handle_connection(message, sender).await
            }
            message @ (RootMessage::DisconnectRemoteSession
            | RootMessage::DisconnectDevice(..)
            | RootMessage::SessionMessageReceived(..)
            | RootMessage::SessionClosed(..)
            | RootMessage::SessionFailed(..)) => self.handle_session(message, sender).await,
            message @ (RootMessage::StopHostedScreenStream(..)
            | RootMessage::ScreenStreamConfigurationFinished { .. }
            | RootMessage::ScreenStreamFinished(..)
            | RootMessage::HostScreenStreamStopFinished { .. }) => {
                self.handle_host_stream(message, sender).await
            }
            message @ (RootMessage::StopCurrentScreenStream
            | RootMessage::PointerMoved(..)
            | RootMessage::Keyboard { .. }
            | RootMessage::MouseButton { .. }
            | RootMessage::VideoFrameReceived(..)
            | RootMessage::VideoFrameReady(..)
            | RootMessage::InitialVideoKeyFrameTimeout { .. }
            | RootMessage::VideoDecoderFinished(..)
            | RootMessage::VideoRendererFailed(..)
            | RootMessage::KeyFrameRequestFinished { .. }
            | RootMessage::OpenRemoteScreen { .. }
            | RootMessage::ScreenStreamRequestFinished { .. }
            | RootMessage::ScreenStreamStopFinished { .. }) => {
                self.handle_remote_video(message, sender).await
            }
        }
    }
}
