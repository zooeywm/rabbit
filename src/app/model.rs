use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    rc::Rc,
};

use eros::Context as _;

use crate::{
    app::App,
    infra::{
        GbmFramePipelineManagerState, KmsScreenCaptureManagerState, NiriScreenLayoutManagerState,
        PendingQuicConnectionRequest, QuicTransportSend, unsync_queue::UnsyncQueue,
    },
    kernel::{
        screen_configuration::{ScreenStreamRequestId, ScreenStreamsConfigured},
        screen_manager::ScreenId,
        session::{SessionId, SessionRole, SessionSend},
        session_control::ScreenInfo,
    },
};

pub(crate) type RabbitApp =
    App<NiriScreenLayoutManagerState, KmsScreenCaptureManagerState, GbmFramePipelineManagerState>;

pub(crate) struct RunningSession {
    pub(crate) key: SessionKey,
    pub(crate) peer_name: Option<String>,
    pub(crate) send: Rc<SessionSend<QuicTransportSend>>,
    pub(crate) screen_streams: HashMap<ScreenId, RunningScreenStream>,
    pub(crate) _receiver: compio::runtime::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey {
    peer_address: SocketAddr,
    role: SessionRole,
}

impl SessionKey {
    pub(crate) fn new(peer_address: SocketAddr, role: SessionRole) -> Self {
        Self { peer_address, role }
    }

    pub(crate) fn peer_address(&self) -> SocketAddr {
        self.peer_address
    }

    pub(crate) fn role(&self) -> SessionRole {
        self.role
    }

    fn matches_controller_target(&self, remote_ip: IpAddr, remote_port: Option<u16>) -> bool {
        self.role == SessionRole::Controller
            && self.peer_address.ip() == remote_ip
            && remote_port.is_none_or(|port| self.peer_address.port() == port)
    }
}

pub(crate) struct RunningScreenStream {
    pub(crate) id: u64,
    pub(crate) cancellation: UnsyncQueue<()>,
    pub(crate) task: Option<compio::runtime::JoinHandle<()>>,
}

impl Drop for RunningScreenStream {
    fn drop(&mut self) {
        self.cancellation.push(());

        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

impl RunningScreenStream {
    pub(crate) fn begin_shutdown(&mut self) -> Option<compio::runtime::JoinHandle<()>> {
        self.cancellation.push(());
        self.task.take()
    }
}

pub(crate) struct ApplicationModel {
    pub(crate) requester_name: String,
    pub(crate) pending_connection_requests: Vec<PendingQuicConnectionRequest>,
    pub(crate) sessions: Vec<RunningSession>,
    pub(crate) remote_screens: HashMap<SessionId, Vec<ScreenInfo>>,
    pub(crate) remote_screen_entries: Vec<(SessionId, ScreenId)>,
    pub(crate) selected_remote_screen: Option<(SessionId, ScreenId)>,
    pub(crate) screen_stream_results: HashMap<SessionId, ScreenStreamsConfigured>,
    next_session_id: u32,
    next_screen_stream_id: u64,
    next_screen_stream_request_id: u32,
    pub(crate) app: RabbitApp,
}

impl ApplicationModel {
    pub(crate) fn new(app: RabbitApp, requester_name: String) -> Self {
        Self {
            app,
            requester_name,
            pending_connection_requests: Vec::new(),
            sessions: Vec::new(),
            remote_screens: HashMap::new(),
            remote_screen_entries: Vec::new(),
            selected_remote_screen: None,
            screen_stream_results: HashMap::new(),
            next_session_id: 0,
            next_screen_stream_id: 0,
            next_screen_stream_request_id: 0,
        }
    }

    pub(crate) fn next_session_id(&mut self) -> eros::Result<SessionId> {
        let id = SessionId(self.next_session_id);
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .context("Failed to allocate a Session ID")?;

        Ok(id)
    }

    pub(crate) fn next_screen_stream_id(&mut self) -> eros::Result<u64> {
        let id = self.next_screen_stream_id;
        self.next_screen_stream_id = self
            .next_screen_stream_id
            .checked_add(1)
            .context("Failed to allocate a screen stream task ID")?;

        Ok(id)
    }

    pub(crate) fn next_screen_stream_request_id(&mut self) -> eros::Result<ScreenStreamRequestId> {
        let id = ScreenStreamRequestId(self.next_screen_stream_request_id);
        self.next_screen_stream_request_id = self
            .next_screen_stream_request_id
            .checked_add(1)
            .context("Failed to allocate a screen stream request ID")?;

        Ok(id)
    }

    pub(crate) fn remove_session(&mut self, id: SessionId) {
        self.sessions.retain(|session| session.send.id() != id);
        self.remote_screens.remove(&id);
        self.screen_stream_results.remove(&id);
    }

    pub(crate) fn has_session(&self, key: &SessionKey) -> bool {
        self.sessions.iter().any(|session| session.key == *key)
    }

    pub(crate) fn has_controller_session(
        &self,
        remote_ip: IpAddr,
        remote_port: Option<u16>,
    ) -> bool {
        self.sessions.iter().any(|session| {
            session
                .key
                .matches_controller_target(remote_ip, remote_port)
        })
    }

    pub(crate) fn begin_screen_stream_shutdown(&mut self) -> Vec<compio::runtime::JoinHandle<()>> {
        let mut tasks = Vec::new();

        for session in &mut self.sessions {
            for stream in session.screen_streams.values_mut() {
                if let Some(task) = stream.begin_shutdown() {
                    tasks.push(task);
                }
            }
        }

        tasks
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use crate::{
        app::model::{RunningScreenStream, SessionKey},
        infra::unsync_queue::UnsyncQueue,
        kernel::session::SessionRole,
    };

    #[test]
    fn screen_stream_shutdown_is_polled_before_the_stream_is_dropped() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let cancellation = UnsyncQueue::default();
            let task_cancellation = cancellation.clone();
            let stopped = Rc::new(Cell::new(false));
            let task_stopped = Rc::clone(&stopped);
            let mut stream = RunningScreenStream {
                id: 0,
                cancellation,
                task: Some(compio::runtime::spawn(async move {
                    task_cancellation.pop().await;
                    task_stopped.set(true);
                })),
            };

            let task = stream
                .begin_shutdown()
                .expect("Running stream should return its task during shutdown");
            task.await
                .expect("Screen stream task should finish after cancellation");

            assert!(stopped.get());
        });
    }

    #[test]
    fn session_key_distinguishes_direction_and_matches_direct_target() {
        let peer_address = "127.0.0.1:52731"
            .parse()
            .expect("Test peer address should be valid");
        let controller = SessionKey::new(peer_address, SessionRole::Controller);
        let host = SessionKey::new(peer_address, SessionRole::Host);

        assert_eq!(
            controller,
            SessionKey::new(peer_address, SessionRole::Controller)
        );
        assert_ne!(controller, host);
        assert!(controller.matches_controller_target(
            "127.0.0.1".parse().expect("Test IP should be valid"),
            None,
        ));
        assert!(controller.matches_controller_target(
            "127.0.0.1".parse().expect("Test IP should be valid"),
            Some(52731),
        ));
        assert!(!controller.matches_controller_target(
            "127.0.0.1".parse().expect("Test IP should be valid"),
            Some(52732),
        ));
        assert!(!host.matches_controller_target(
            "127.0.0.1".parse().expect("Test IP should be valid"),
            None,
        ));
    }
}
