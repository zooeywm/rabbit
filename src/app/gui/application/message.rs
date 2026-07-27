use std::net::SocketAddr;

use crate::{
    app::{gui::state::WorkspaceSection, services::host_stream::HostStreamPlan},
    infra::{
        DirectConnectionOutcome, PendingConnectionRequest, SessionTransport, SessionTransportRecv,
        SessionTransportSend, unsync_queue::UnsyncQueue,
    },
    kernel::{
        geometry::PixelSize,
        screen_configuration::ScreenStreamRequestId,
        screen_manager::ScreenId,
        session::{ReceivedVideoFrame, SessionId, SessionMessage, SessionRecv, SessionSend},
    },
};

pub(super) struct PendingHostSession {
    pub(super) peer_address: SocketAddr,
    pub(super) peer_name: String,
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
    },
    DisconnectRemoteSession,
    StopHostedScreenStream(usize),
    DisconnectDevice(usize),
    ResetDirectConnection,
    StopCurrentScreenStream,
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
}
