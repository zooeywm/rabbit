//! Application root: session orchestration, message loop, and grouped UI/runtime state.

mod message;
mod tasks;
mod update;
mod video_decoder;
mod view_state;

use std::{collections::HashSet, marker::PhantomData, net::SocketAddr, rc::Rc};

use eros::Context as _;
use futures_util::{future::Either, pin_mut};
use tracing::{error, info, warn};

use crate::app::{
    gui::{
        application::{
            message::{MessageSender, RootMessage},
            tasks::{receive_connection_requests, receive_session},
            video_decoder::{RunningVideoDecoder, VideoDecoderInput},
        },
        state::{DirectConnectionState, ScreenStreamState, WorkspaceSection},
        view::{GuiIntent, ViewPublisher},
    },
    model::{ApplicationModel, RunningScreenStream, RunningSession, SessionKey},
    services::{
        host_stream::{HostStreamPlan, plan_preserved_streams},
        session_catalog,
    },
};

use crate::{
    app::{
        LoggerGuard,
        config::Config,
        init_logging,
        platform::{ApplicationStack, RemoteVideoStack, RunnableApp},
        screen_stream::run_host_screen_stream,
    },
    infra::{
        ConnectionEndpoint, PendingConnectionRequest, SessionTransportSend, WorkerReaper,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        geometry::FrameRate,
        protocol::{PROTOCOL_NAME, protocol_version_string},
        screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{SessionId, SessionRecv, SessionRole, SessionSend},
        transport::TransportRecv,
    },
};

/// Lifecycle flags for the application message loop.
struct LifecycleState {
    closing: bool,
    finished: bool,
}

/// Workspace navigation and status copy shown in the shell.
struct WorkspaceState {
    active_section: WorkspaceSection,
    status_message: String,
    stream_settings_error: String,
}

/// Local listener identity and outbound direct-connection UI state.
struct ListenerState {
    local_protocol: &'static str,
    local_port: u16,
    online: bool,
    direct_connection: DirectConnectionState,
    _connection_listener: compio::runtime::JoinHandle<()>,
}

/// Controller-side remote stream and decoder presentation state.
struct RemoteStreamState<Video>
where
    Video: RemoteVideoStack,
{
    screen_stream: ScreenStreamState,
    video_decoder: Option<RunningVideoDecoder<Video>>,
}

/// Host-side pending stream start/stop acknowledgements.
struct HostStreamState {
    pending_starts: HashSet<(SessionId, ScreenId)>,
    pending_stops: HashSet<(SessionId, ScreenId)>,
}

/// Root application state and async message loop.
struct RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    model: ApplicationModel<Stack>,
    view: ViewPublisher<Stack::RemoteVideoViewStack>,
    messages: UnsyncQueue<RootMessage>,
    lifecycle: LifecycleState,
    workspace: WorkspaceState,
    listener: ListenerState,
    remote_stream: RemoteStreamState<Stack::RemoteVideo>,
    host_stream: HostStreamState,
    _logger_guard: LoggerGuard,
}

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    pub(super) fn start_video_decoder(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        sender: &MessageSender,
    ) -> eros::Result<()> {
        if self
            .remote_stream
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.matches(session_id, screen_id))
        {
            return Ok(());
        }

        self.stop_video_decoder()?;
        let input = UnsyncQueue::default();
        let receiver = input.clone();
        let view = self.view.clone();
        let finished = sender.clone();
        let enable_probing = <Stack::App as AsRef<Config>>::as_ref(&self.model.app)
            .video
            .enable_client_probing;
        let inputs = Box::pin(futures_util::stream::unfold(
            receiver,
            |receiver| async move {
                match receiver.pop().await {
                    VideoDecoderInput::Frame(frame) => Some((Ok(frame), receiver)),
                    VideoDecoderInput::Shutdown => None,
                }
            },
        ));
        let task = compio::runtime::spawn(async move {
            let result = <Stack::RemoteVideo as RemoteVideoStack>::run_decoder(
                inputs,
                move |frame| std::future::ready(view.present_video(session_id, screen_id, frame)),
                enable_probing,
            )
            .await;
            finished.post(RootMessage::VideoDecoderFinished(
                session_id, screen_id, result,
            ));
        });
        self.remote_stream.video_decoder = Some(RunningVideoDecoder {
            session_id,
            screen_id,
            input,
            task: Some(task),
            video: PhantomData,
        });
        Ok(())
    }

    pub(super) fn stop_video_decoder(&mut self) -> eros::Result<()> {
        self.remote_stream.video_decoder = None;
        self.view.clear_video()
    }

    pub(super) fn stop_session_video_decoder(&mut self, session_id: SessionId) -> eros::Result<()> {
        if self
            .remote_stream
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.session_id == session_id)
        {
            self.stop_video_decoder()?;
        }
        Ok(())
    }

    pub(super) fn configure_preserved_screens(
        &self,
        request: SetScreenStreams,
    ) -> (ScreenStreamsConfigured, Vec<HostStreamPlan>) {
        plan_preserved_streams(request, |id| self.model.app.screen(id).cloned())
    }

    pub(super) fn start_session<R>(
        &mut self,
        peer_address: SocketAddr,
        peer_name: Option<String>,
        send: SessionSend<SessionTransportSend>,
        recv: SessionRecv<R>,
        sender: &MessageSender,
    ) -> bool
    where
        R: TransportRecv + 'static,
    {
        let key = SessionKey::new(peer_address, send.role());
        if self.model.has_session(&key) {
            warn!(
                event = "duplicate_session_rejected",
                %peer_address,
                role = ?send.role(),
                "Duplicate Session rejected"
            );
            compio::runtime::spawn(async move {
                send.close().await;
            })
            .detach();

            return false;
        }

        let session_id = send.id();
        let role = send.role();
        let mut session = RunningSession::new(
            key,
            peer_name,
            Rc::new(send),
            compio::runtime::spawn(receive_session(recv, sender.clone())),
        );
        let activated = session.activate();
        info!(
            event = "session_started",
            session_id = session_id.0,
            role = ?role,
            phase = ?session.phase,
            activated,
            "Session started"
        );
        self.model.sessions.push(session);

        true
    }

    pub(super) fn replace_screen_stream(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
        frame_rate: FrameRate,
        sender: &MessageSender,
    ) -> eros::Result<()> {
        let frames = FramePipelineManager::subscribe(
            &mut self.model.app,
            &screen_id,
            parameters,
            frame_rate,
        )?;
        let stream_id = self.model.next_screen_stream_id()?;
        let Some(session) = self
            .model
            .sessions
            .iter_mut()
            .find(|session| session.send.id() == session_id)
        else {
            eros::bail!(
                "Session {} closed before screen {} stream could start",
                session_id.0,
                screen_id.0
            );
        };
        if !session.admits_new_streams() {
            eros::bail!(
                "Session {} is {:?} and cannot start screen {} stream",
                session_id.0,
                session.phase,
                screen_id.0
            );
        }
        let session_send = Rc::clone(&session.send);
        let cancellation = UnsyncQueue::default();
        let task_cancellation = cancellation.clone();
        let encoder_commands = UnsyncQueue::default();
        let task_encoder_commands = encoder_commands.clone();
        let task_sender = sender.clone();
        let task = compio::runtime::spawn(async move {
            let result = run_host_screen_stream::<_, _, Stack::ScreenStreamEncoder>(
                frames,
                screen_id,
                session_send,
                task_cancellation,
                task_encoder_commands,
                frame_rate,
            )
            .await;
            task_sender.post(RootMessage::ScreenStreamFinished(
                session_id, screen_id, stream_id, result,
            ));
        });

        session.screen_streams.insert(
            screen_id,
            RunningScreenStream {
                id: stream_id,
                cancellation,
                encoder_commands,
                task: Some(task),
            },
        );

        Ok(())
    }

    pub(super) fn remove_session(&mut self, id: SessionId) {
        let was_controller = self.model.sessions.iter().any(|session| {
            session.send.id() == id && session.key.role() == SessionRole::Controller
        });
        self.model.remove_session(id);
        self.host_stream
            .pending_starts
            .retain(|(session_id, _)| *session_id != id);
        self.host_stream
            .pending_stops
            .retain(|(session_id, _)| *session_id != id);
        if was_controller {
            self.listener.direct_connection.reset();
        }
        self.refresh_remote_screen_list();
    }

    pub(super) fn refresh_remote_screen_list(&mut self) {
        session_catalog::rebuild_remote_screen_entries(&mut self.model);
    }

    pub(super) fn set_connection_status(&mut self, status: impl Into<String>) {
        self.workspace.status_message = status.into();
    }

    pub(super) fn take_connection_request(
        &mut self,
        index: usize,
    ) -> Option<PendingConnectionRequest> {
        if index >= self.model.pending_connection_requests.len() {
            return None;
        }

        Some(self.model.pending_connection_requests.remove(index))
    }

    pub(super) fn host_session_ids(&self) -> Vec<SessionId> {
        session_catalog::host_session_ids(&self.model)
    }

    pub(super) fn hosted_screen_stream_entries(&self) -> Vec<(SessionId, ScreenId)> {
        session_catalog::hosted_screen_stream_entries(&self.model)
    }

    pub(super) fn controller_session_id(&self) -> Option<SessionId> {
        let DirectConnectionState::Connected { peer } = &self.listener.direct_connection else {
            return None;
        };
        session_catalog::controller_session_for_peer(&self.model, *peer)
    }

    pub(super) fn disconnect_session(&mut self, session_id: SessionId) -> eros::Result<bool> {
        let Some(session) = self
            .model
            .sessions
            .iter_mut()
            .find(|session| session.send.id() == session_id)
        else {
            return Ok(false);
        };

        session.begin_drain();
        let send = Rc::clone(&session.send);
        let tasks = session
            .screen_streams
            .values_mut()
            .filter_map(RunningScreenStream::begin_shutdown)
            .collect::<Vec<_>>();
        if self
            .remote_stream
            .screen_stream
            .active_screen()
            .is_some_and(|(active_session_id, _)| active_session_id == session_id)
        {
            self.remote_stream.screen_stream.reset();
        }
        self.stop_session_video_decoder(session_id)?;

        self.remove_session(session_id);
        compio::runtime::spawn(async move {
            for task in tasks {
                if let Err(error) = task.await {
                    error!(
                        session_id = session_id.0,
                        error = ?error,
                        "Screen stream task failed while disconnecting Session"
                    );
                }
            }
            send.close().await;
        })
        .detach();

        info!(
            event = "session_disconnect_requested",
            session_id = session_id.0,
            "Session disconnect requested"
        );
        self.set_connection_status(format!("Disconnected Session {}", session_id.0));
        Ok(true)
    }
}

impl<Stack> RootApplication<Stack>
where
    Stack: ApplicationStack,
{
    async fn new(
        config: Config,
        view: ViewPublisher<Stack::RemoteVideoViewStack>,
        messages: UnsyncQueue<RootMessage>,
        sender: &MessageSender,
    ) -> eros::Result<Self> {
        let logger_guard = init_logging(&config)?;
        let (worker_reaper, worker_reaper_handle) =
            WorkerReaper::new().context("Failed to start the background worker reaper")?;
        let connection_endpoint = ConnectionEndpoint::new(config.network.transport)
            .await
            .context("Failed to create the configured connection endpoint")?;
        let local_address = connection_endpoint.local_address()?;
        let local_protocol = config.network.transport.listener_protocol();
        let requester_name = format!("{} ({})", config.app_name, local_address.port());

        info!(
            event = "listener_started",
            transport = ?config.network.transport,
            %local_address,
            protocol = PROTOCOL_NAME,
            protocol_version = protocol_version_string(),
            "Connection listener started"
        );

        let app = Stack::create_app(
            config,
            connection_endpoint.clone(),
            worker_reaper,
            worker_reaper_handle,
        )?;
        // Run the generic App lifecycle after the platform stack has selected and
        // constructed the concrete App<...> type.
        let mut app = app;
        app.run_app().await?;

        Ok(Self {
            model: ApplicationModel::new(app, requester_name),
            view,
            messages,
            lifecycle: LifecycleState {
                closing: false,
                finished: false,
            },
            workspace: WorkspaceState {
                active_section: WorkspaceSection::default(),
                status_message: String::new(),
                stream_settings_error: String::new(),
            },
            listener: ListenerState {
                local_protocol,
                local_port: local_address.port(),
                online: true,
                direct_connection: DirectConnectionState::default(),
                _connection_listener: compio::runtime::spawn(receive_connection_requests(
                    connection_endpoint,
                    sender.clone(),
                )),
            },
            remote_stream: RemoteStreamState {
                screen_stream: ScreenStreamState::default(),
                video_decoder: None,
            },
            host_stream: HostStreamState {
                pending_starts: HashSet::new(),
                pending_stops: HashSet::new(),
            },
            _logger_guard: logger_guard,
        })
    }

    async fn run(
        config: Config,
        view: ViewPublisher<Stack::RemoteVideoViewStack>,
        intents: flume::Receiver<GuiIntent>,
    ) -> eros::Result<()> {
        let messages = UnsyncQueue::default();
        let sender = MessageSender::new(messages.clone());
        let mut application = Self::new(config, view, messages, &sender).await?;
        application.publish_view_state()?;

        while !application.lifecycle.finished {
            let message = application.next_message(&intents).await;
            let changed = application.update(message, &sender).await?;
            if changed {
                application.publish_view_state()?;
            }
        }

        application.view.quit()?;
        Ok(())
    }

    pub(super) async fn next_message(&self, intents: &flume::Receiver<GuiIntent>) -> RootMessage {
        let internal = self.messages.pop();
        let gui = intents.recv_async();
        pin_mut!(internal, gui);

        match futures_util::future::select(internal, gui).await {
            Either::Left((message, _)) => message,
            Either::Right((Ok(intent), _)) => match intent {
                GuiIntent::SelectSection(section) => RootMessage::SelectSection(section),
                GuiIntent::Connect(address) => RootMessage::ConnectDirect(address),
                GuiIntent::DecideConnectionRequest { index, accept } => {
                    if accept {
                        RootMessage::AcceptConnectionRequest(index)
                    } else {
                        RootMessage::RejectConnectionRequest(index)
                    }
                }
                GuiIntent::OpenRemoteScreen {
                    index,
                    width,
                    height,
                    frame_rate,
                } => RootMessage::OpenRemoteScreen {
                    selected_index: index,
                    width,
                    height,
                    frame_rate,
                },
                GuiIntent::DisconnectRemoteSession => RootMessage::DisconnectRemoteSession,
                GuiIntent::StopHostedScreenStream(index) => {
                    RootMessage::StopHostedScreenStream(index)
                }
                GuiIntent::DisconnectDevice(index) => RootMessage::DisconnectDevice(index),
                GuiIntent::RetryConnection => RootMessage::ResetDirectConnection,
                GuiIntent::StopScreenStream => RootMessage::StopCurrentScreenStream,
                GuiIntent::VideoFrameReady {
                    session_id,
                    screen_id,
                } => RootMessage::VideoFrameReady(session_id, screen_id),
                GuiIntent::VideoRendererFailed(error) => RootMessage::VideoRendererFailed(error),
                GuiIntent::Close => RootMessage::Close,
            },
            Either::Right((Err(_), _)) => RootMessage::Close,
        }
    }
}

pub(super) async fn run_root<Stack>(
    config: Config,
    view: ViewPublisher<Stack::RemoteVideoViewStack>,
    intents: flume::Receiver<GuiIntent>,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
{
    RootApplication::<Stack>::run(config, view, intents).await
}
