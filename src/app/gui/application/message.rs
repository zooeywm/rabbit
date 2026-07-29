use std::net::SocketAddr;

use crate::{
    app::{
        gui::{state::WorkspaceSection, view::PointerViewportEvent},
        runtime::host_control::{absolute_pointer_queue_key, can_replace_queued_absolute_pointer},
        services::host_stream::HostStreamPlan,
    },
    infra::{
        DirectConnectionOutcome, PendingConnectionRequest, SessionTransport, SessionTransportRecv,
        SessionTransportSend, unsync_queue::UnsyncQueue,
    },
    kernel::{
        connection_request::PeerCapabilities,
        geometry::PixelSize,
        input::{InputState, KeyboardKey, MouseButton},
        screen_configuration::ScreenStreamRequestId,
        screen_manager::ScreenId,
        session::{ReceivedVideoFrame, SessionId, SessionMessage, SessionRecv, SessionSend},
    },
};

pub(super) struct PendingHostSession {
    pub(super) peer_address: SocketAddr,
    pub(super) peer_name: String,
    pub(super) peer_capabilities: PeerCapabilities,
    pub(super) send: SessionSend<SessionTransportSend>,
    pub(super) recv: SessionRecv<SessionTransportRecv>,
}

pub(super) enum RootMessage {
    SelectSection(WorkspaceSection),
    Close,
    ShutdownFinished,
    ConnectDirect(String),
    DirectConnectionFinished(eros::Result<DirectConnectionOutcome>),
    ConnectionRequest(PendingConnectionRequest),
    AcceptConnectionRequest(usize),
    RejectConnectionRequest(usize),
    ConnectionAccepted {
        peer_name: String,
        peer_capabilities: PeerCapabilities,
        result: eros::Result<SessionTransport>,
    },
    InitialScreenListFinished {
        session: PendingHostSession,
        result: eros::Result<()>,
    },
    ConnectionRejected(eros::Result<()>),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
    SessionMessageReceived(SessionId, SessionMessage),
    VideoFrameReceived(SessionId, ReceivedVideoFrame),
    VideoFrameReady(SessionId, ScreenId),
    InitialVideoKeyFrameTimeout {
        request_id: ScreenStreamRequestId,
        session_id: SessionId,
        screen_id: ScreenId,
    },
    VideoDecoderFinished(SessionId, ScreenId, eros::Result<()>),
    VideoRendererFailed(String),
    ScreenStreamConfigurationFinished {
        session_id: SessionId,
        streams: Vec<HostStreamPlan>,
        result: eros::Result<()>,
    },
    ScreenStreamRequestFinished {
        request_id: ScreenStreamRequestId,
        session_id: SessionId,
        screen_id: ScreenId,
        frame_size: PixelSize,
        result: eros::Result<()>,
    },
    ScreenStreamStopFinished {
        session_id: SessionId,
        screen_id: ScreenId,
        result: eros::Result<()>,
    },
    KeyFrameRequestFinished {
        session_id: SessionId,
        screen_id: ScreenId,
        result: eros::Result<()>,
    },
    HostScreenStreamStopFinished {
        session_id: SessionId,
        screen_id: ScreenId,
        result: eros::Result<()>,
    },
    SessionClosed(SessionId),
    SessionFailed(SessionId, eros::ErrorUnion),
    ScreenStreamFinished(SessionId, ScreenId, u64, eros::Result<()>),
    OpenRemoteScreen {
        selected_index: usize,
        width: String,
        height: String,
        frame_rate: String,
        dynamic_frame_rate: bool,
        bitrate_mbps: String,
    },
    DisconnectRemoteSession,
    StopHostedScreenStream(usize),
    DisconnectDevice(usize),
    ResetDirectConnection,
    StopCurrentScreenStream,
    PointerMoved(PointerViewportEvent),
    Keyboard {
        key: KeyboardKey,
        state: InputState,
        repeat: bool,
    },
    MouseButton {
        button: MouseButton,
        state: InputState,
    },
}

#[derive(Clone)]
pub(super) struct MessageSender {
    messages: UnsyncQueue<RootMessage>,
}

impl MessageSender {
    pub(super) fn new(messages: UnsyncQueue<RootMessage>) -> Self {
        Self { messages }
    }

    pub(super) fn post(&self, message: RootMessage) {
        self.messages.push(message);
    }

    pub(super) fn post_session_message(&self, session_id: SessionId, message: SessionMessage) {
        let incoming_key = absolute_pointer_queue_key(session_id, &message);
        self.messages.push_or_replace_back(
            RootMessage::SessionMessageReceived(session_id, message),
            |queued| {
                let RootMessage::SessionMessageReceived(queued_session_id, queued_message) = queued
                else {
                    return false;
                };
                can_replace_queued_absolute_pointer(
                    incoming_key,
                    *queued_session_id,
                    queued_message,
                )
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::unsync_queue::UnsyncQueue,
        kernel::{
            input::{
                AbsolutePointerMove, InputState, MouseButton, MouseButtonInput, NormalizedPosition,
                RemoteInputEvent,
            },
            screen_manager::ScreenId,
            session::{SessionId, SessionMessage},
            session_control::ControlMessage,
        },
    };

    use super::{MessageSender, RootMessage};

    fn absolute(x: u16) -> SessionMessage {
        SessionMessage::Control(ControlMessage::RemoteInput(
            RemoteInputEvent::AbsolutePointerMove(AbsolutePointerMove {
                screen_id: ScreenId(4),
                position: NormalizedPosition { x, y: 20 },
            }),
        ))
    }

    fn left_button() -> SessionMessage {
        SessionMessage::Control(ControlMessage::RemoteInput(RemoteInputEvent::MouseButton(
            MouseButtonInput {
                screen_id: ScreenId(4),
                button: MouseButton::Left,
                state: InputState::Pressed,
            },
        )))
    }

    fn absolute_x(message: RootMessage) -> u16 {
        let RootMessage::SessionMessageReceived(
            _,
            SessionMessage::Control(ControlMessage::RemoteInput(
                RemoteInputEvent::AbsolutePointerMove(movement),
            )),
        ) = message
        else {
            panic!("queued message should be an absolute pointer move");
        };
        movement.position.x
    }

    #[test]
    fn coalesces_absolute_moves_without_crossing_reliable_input_barriers() {
        let queue = UnsyncQueue::default();
        let sender = MessageSender::new(queue.clone());
        let session_id = SessionId(3);

        sender.post_session_message(session_id, absolute(1));
        sender.post_session_message(session_id, absolute(2));
        sender.post_session_message(session_id, left_button());
        sender.post_session_message(session_id, absolute(3));
        sender.post_session_message(session_id, absolute(4));

        assert_eq!(absolute_x(queue.try_pop().expect("first movement")), 2);
        assert!(matches!(
            queue.try_pop(),
            Some(RootMessage::SessionMessageReceived(
                _,
                SessionMessage::Control(ControlMessage::RemoteInput(
                    RemoteInputEvent::MouseButton(_)
                ))
            ))
        ));
        assert_eq!(absolute_x(queue.try_pop().expect("second movement")), 4);
        assert!(queue.try_pop().is_none());
    }
}
