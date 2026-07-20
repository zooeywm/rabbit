use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    rc::Rc,
};

use eros::Context;
use futures_util::{StreamExt as _, future::Either, pin_mut};
use tracing::{error, info, trace, warn};

use crate::app::{
    gui::{
        state::{
            ConnectedDeviceView, ConnectionRequestView, DirectConnectionCompletion,
            DirectConnectionState, DirectTarget, HostedScreenStreamView, RemoteScreenView,
            ScreenStreamState, ScreenStreamTarget, ViewPage, ViewState, WorkspaceSection,
        },
        view::{Gui, GuiIntent, ViewPublisher},
    },
    model::{ApplicationModel, LatestVideoFrames, RunningScreenStream, RunningSession, SessionKey},
};

use crate::{
    app::{App, LoggerGuard, config::Config, init_logging, screen_stream::run_host_screen_stream},
    infra::{
        DirectConnectionOutcome, GStreamerVideoDecoder, PendingQuicConnectionRequest, QuicEndpoint,
        QuicTransport, QuicTransportRecv, QuicTransportSend, WorkerReaper, connect_transport,
        create_frame_pipeline_manager_state, create_screen_capture_manager_state,
        create_screen_layout_manager_state, receive_request, unsync_queue::UnsyncQueue,
    },
    kernel::{
        connection_request::ConnectionRequest,
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        geometry::PixelSize,
        screen_configuration::{
            RemoteDisplayMode, ResolutionResult, ScreenResolutionOutcome, ScreenResolutionStatus,
            ScreenStreamRequest, ScreenStreamRequestId, ScreenStreamsConfigured, SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{Session, SessionId, SessionMessage, SessionRecv, SessionRole, SessionSend},
        session_control::{ControlMessage, OutgoingScreenList},
        transport::TransportRecv,
        video_decoder::VideoDecoder,
    },
};

mod state;
mod video_view;
mod view;

pub(crate) fn run() -> eros::Result<()> {
    let (gui, publisher, intents) = Gui::new()?;
    let thread_publisher = publisher.clone();
    let application_thread = std::thread::Builder::new()
        .name("rabbit-app".to_string())
        .spawn(move || {
            let result = (|| {
                let runtime = compio::runtime::Runtime::new()
                    .context("Failed to create the Rabbit Compio runtime")?;
                runtime.block_on(RootApplication::run(thread_publisher.clone(), intents))
            })();

            if result.is_err()
                && let Err(error) = thread_publisher.quit()
            {
                eprintln!("Failed to stop the Slint event loop after an App error: {error}");
            }
            result
        })
        .context("Failed to start the Rabbit App thread")?;

    let gui_result = gui.run();
    gui.request_close();
    let application_result = match application_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Rabbit App thread terminated unexpectedly"),
    };

    gui_result?;
    application_result
}

pub(crate) struct PendingHostSession {
    peer_address: SocketAddr,
    peer_name: String,
    send: SessionSend<QuicTransportSend>,
    recv: SessionRecv<QuicTransportRecv>,
}

pub(crate) struct RootApplication {
    model: ApplicationModel,
    view: ViewPublisher,
    messages: UnsyncQueue<RootMessage>,
    closing: bool,
    finished: bool,
    local_port: u16,
    listener_online: bool,
    active_section: WorkspaceSection,
    status_message: String,
    direct_connection: DirectConnectionState,
    screen_stream: ScreenStreamState,
    video_decoder: Option<RunningVideoDecoder>,
    pending_screen_stream_starts: HashSet<(SessionId, ScreenId)>,
    pending_host_screen_stream_stops: HashSet<(SessionId, ScreenId)>,
    _connection_listener: compio::runtime::JoinHandle<()>,
    _logger_guard: LoggerGuard,
}

pub(crate) enum RootMessage {
    SelectSection(WorkspaceSection),
    Close,
    ShutdownFinished,
    ConnectDirect(String),
    DirectConnectionFinished(eros::Result<DirectConnectionOutcome>),
    ConnectionRequest(PendingQuicConnectionRequest),
    AcceptConnectionRequest(usize),
    RejectConnectionRequest(usize),
    ConnectionAccepted {
        peer_name: String,
        result: eros::Result<QuicTransport>,
    },
    InitialScreenListFinished {
        session: PendingHostSession,
        result: eros::Result<()>,
    },
    ConnectionRejected(eros::Result<()>),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
    SessionMessageReceived(SessionId, SessionMessage),
    VideoFrameAvailable(SessionId, ScreenId),
    VideoFramePresented(SessionId, ScreenId),
    VideoDecoderFinished(SessionId, ScreenId, eros::Result<()>),
    VideoRendererFailed(String),
    ScreenStreamConfigurationFinished {
        session_id: SessionId,
        streams: Vec<(ScreenId, FramePipelineParameters)>,
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
    HostScreenStreamStopFinished {
        session_id: SessionId,
        screen_id: ScreenId,
        result: eros::Result<()>,
    },
    SessionClosed(SessionId),
    SessionFailed(SessionId, eros::ErrorUnion),
    ScreenStreamFinished(SessionId, ScreenId, u64, eros::Result<()>),
    OpenRemoteScreen(usize),
    DisconnectRemoteSession,
    StopHostedScreenStream(usize),
    DisconnectDevice(usize),
    ResetDirectConnection,
    StopCurrentScreenStream,
}

struct RunningVideoDecoder {
    session_id: SessionId,
    screen_id: ScreenId,
    input: flume::Sender<crate::kernel::session::ReceivedVideoFrame>,
    stale: flume::Receiver<crate::kernel::session::ReceivedVideoFrame>,
    task: Option<compio::runtime::JoinHandle<()>>,
}

impl RunningVideoDecoder {
    fn publish(&self, mut frame: crate::kernel::session::ReceivedVideoFrame) -> eros::Result<()> {
        loop {
            match self.input.try_send(frame) {
                Ok(()) => return Ok(()),
                Err(flume::TrySendError::Full(returned)) => {
                    frame = returned;
                    match self.stale.try_recv() {
                        Ok(_) | Err(flume::TryRecvError::Empty) => {}
                        Err(flume::TryRecvError::Disconnected) => {
                            eros::bail!("Video decoder input disconnected")
                        }
                    }
                }
                Err(flume::TrySendError::Disconnected(_)) => {
                    eros::bail!("Video decoder input disconnected")
                }
            }
        }
    }

    fn matches(&self, session_id: SessionId, screen_id: ScreenId) -> bool {
        self.session_id == session_id && self.screen_id == screen_id
    }
}

impl Drop for RunningVideoDecoder {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

#[derive(Clone)]
struct MessageSender {
    messages: UnsyncQueue<RootMessage>,
}

impl MessageSender {
    fn post(&self, message: RootMessage) {
        self.messages.push(message);
    }
}

impl RootApplication {
    fn start_video_decoder(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        sender: &MessageSender,
    ) -> eros::Result<()> {
        if self
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.matches(session_id, screen_id))
        {
            return Ok(());
        }

        self.stop_video_decoder()?;
        let (input, receiver) = flume::bounded(1);
        let stale = receiver.clone();
        let view = self.view.clone();
        let finished = sender.clone();
        let inputs = receiver.into_stream().map(Ok);
        let task = compio::runtime::spawn(async move {
            let result = GStreamerVideoDecoder::run(inputs, move |frame| {
                std::future::ready(view.present_video(session_id, screen_id, frame))
            })
            .await;
            finished.post(RootMessage::VideoDecoderFinished(
                session_id, screen_id, result,
            ));
        });
        self.video_decoder = Some(RunningVideoDecoder {
            session_id,
            screen_id,
            input,
            stale,
            task: Some(task),
        });
        Ok(())
    }

    fn stop_video_decoder(&mut self) -> eros::Result<()> {
        self.video_decoder = None;
        self.view.clear_video()
    }

    fn stop_session_video_decoder(&mut self, session_id: SessionId) -> eros::Result<()> {
        if self
            .video_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.session_id == session_id)
        {
            self.stop_video_decoder()?;
        }
        Ok(())
    }

    fn configure_preserved_screens(
        &self,
        request: SetScreenStreams,
    ) -> (
        ScreenStreamsConfigured,
        Vec<(ScreenId, FramePipelineParameters)>,
    ) {
        let SetScreenStreams {
            request_id,
            desired_streams,
        } = request;
        let mut streams = Vec::new();
        let outcomes = desired_streams
            .into_iter()
            .map(|desired_stream| {
                let status = match self.model.app.screen(&desired_stream.screen_id) {
                    Some(screen) => match desired_stream.remote_display {
                        RemoteDisplayMode::Preserve => {
                            streams.push((
                                desired_stream.screen_id,
                                FramePipelineParameters {
                                    frame_size: desired_stream.frame_size,
                                },
                            ));
                            ScreenResolutionStatus::Configured(ResolutionResult::Preserved {
                                requested: desired_stream.frame_size,
                                actual: screen.resolution,
                            })
                        }
                    },
                    None => ScreenResolutionStatus::Failed {
                        requested: desired_stream.frame_size,
                        actual: None,
                    },
                };

                ScreenResolutionOutcome {
                    screen_id: desired_stream.screen_id,
                    status,
                }
            })
            .collect();

        (
            ScreenStreamsConfigured {
                request_id,
                outcomes,
            },
            streams,
        )
    }

    fn start_session<R>(
        &mut self,
        peer_address: SocketAddr,
        peer_name: Option<String>,
        send: SessionSend<QuicTransportSend>,
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

        let received_video_frames = LatestVideoFrames::default();
        info!(
            event = "session_started",
            session_id = send.id().0,
            role = ?send.role(),
            "Session started"
        );
        self.model.sessions.push(RunningSession {
            key,
            peer_name,
            send: Rc::new(send),
            screen_streams: Default::default(),
            received_video_frames: received_video_frames.clone(),
            _receiver: compio::runtime::spawn(receive_session(
                recv,
                received_video_frames,
                sender.clone(),
            )),
        });

        true
    }

    fn replace_screen_stream(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
        sender: &MessageSender,
    ) -> eros::Result<()> {
        let frames = FramePipelineManager::subscribe(&mut self.model.app, &screen_id, parameters)?;
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
        let session_send = Rc::clone(&session.send);
        let cancellation = UnsyncQueue::default();
        let task_cancellation = cancellation.clone();
        let task_sender = sender.clone();
        let task = compio::runtime::spawn(async move {
            let result = run_host_screen_stream::<_, _, crate::infra::GStreamerVideoEncoder>(
                frames,
                screen_id,
                session_send,
                task_cancellation,
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
                task: Some(task),
            },
        );

        Ok(())
    }

    fn remove_session(&mut self, id: SessionId) {
        let was_controller = self.model.sessions.iter().any(|session| {
            session.send.id() == id && session.key.role() == SessionRole::Controller
        });
        self.model.remove_session(id);
        self.pending_screen_stream_starts
            .retain(|(session_id, _)| *session_id != id);
        self.pending_host_screen_stream_stops
            .retain(|(session_id, _)| *session_id != id);
        if was_controller {
            self.direct_connection.reset();
        }
        self.refresh_remote_screen_list();
    }

    fn refresh_remote_screen_list(&mut self) {
        let mut entries = self
            .model
            .remote_screens
            .iter()
            .flat_map(|(session_id, screens)| screens.iter().map(|screen| (*session_id, screen.id)))
            .collect::<Vec<_>>();

        entries.sort_by_key(|(session_id, screen_id)| (session_id.0, screen_id.0));
        self.model.remote_screen_entries.clear();
        self.model.remote_screen_entries.extend(entries);
        self.model.selected_remote_screen = None;
    }

    fn set_connection_status(&mut self, status: impl Into<String>) {
        self.status_message = status.into();
    }

    fn parse_direct_target(input: &str) -> eros::Result<(IpAddr, Option<u16>)> {
        let input = input.trim();

        if let Ok(address) = input.parse::<SocketAddr>() {
            return Ok((address.ip(), Some(address.port())));
        }

        let ip = input
            .parse::<IpAddr>()
            .with_context(|| format!("Failed to parse direct connection IP {input:?}"))?;

        Ok((ip, None))
    }

    fn take_connection_request(&mut self, index: usize) -> Option<PendingQuicConnectionRequest> {
        if index >= self.model.pending_connection_requests.len() {
            return None;
        }

        Some(self.model.pending_connection_requests.remove(index))
    }

    fn host_session_ids(&self) -> Vec<SessionId> {
        let mut sessions = self
            .model
            .sessions
            .iter()
            .filter(|session| session.key.role() == SessionRole::Host)
            .map(|session| (session.key.peer_address(), session.send.id()))
            .collect::<Vec<_>>();
        sessions.sort_by_key(|(address, session_id)| (*address, session_id.0));
        sessions
            .into_iter()
            .map(|(_, session_id)| session_id)
            .collect()
    }

    fn hosted_screen_stream_entries(&self) -> Vec<(SessionId, ScreenId)> {
        let mut streams =
            self.model
                .sessions
                .iter()
                .filter(|session| session.key.role() == SessionRole::Host)
                .flat_map(|session| {
                    session.screen_streams.keys().map(|screen_id| {
                        (session.key.peer_address(), session.send.id(), *screen_id)
                    })
                })
                .collect::<Vec<_>>();
        streams
            .sort_by_key(|(address, session_id, screen_id)| (*address, session_id.0, screen_id.0));
        streams
            .into_iter()
            .map(|(_, session_id, screen_id)| (session_id, screen_id))
            .collect()
    }

    fn controller_session_id(&self) -> Option<SessionId> {
        let DirectConnectionState::Connected { peer } = &self.direct_connection else {
            return None;
        };

        self.model
            .sessions
            .iter()
            .find(|session| {
                session.key.role() == SessionRole::Controller && session.key.peer_address() == *peer
            })
            .map(|session| session.send.id())
    }

    fn disconnect_session(&mut self, session_id: SessionId) -> eros::Result<bool> {
        let Some(session) = self
            .model
            .sessions
            .iter_mut()
            .find(|session| session.send.id() == session_id)
        else {
            return Ok(false);
        };

        let send = Rc::clone(&session.send);
        let tasks = session
            .screen_streams
            .values_mut()
            .filter_map(RunningScreenStream::begin_shutdown)
            .collect::<Vec<_>>();
        if self
            .screen_stream
            .active_screen()
            .is_some_and(|(active_session_id, _)| active_session_id == session_id)
        {
            self.screen_stream.reset();
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

impl RootApplication {
    async fn new(
        view: ViewPublisher,
        messages: UnsyncQueue<RootMessage>,
        sender: &MessageSender,
    ) -> eros::Result<Self> {
        let config = Config::new()?;
        let logger_guard = init_logging(&config)?;
        let (worker_reaper, worker_reaper_handle) =
            WorkerReaper::new().context("Failed to start the background worker reaper")?;
        let screen_layout_manager_state = create_screen_layout_manager_state()
            .context("Failed to create the screen layout manager state")?;
        let screen_capture_manager_state = create_screen_capture_manager_state(
            config.video.enable_probing,
            worker_reaper_handle.clone(),
        );
        let frame_pipeline_manager_state =
            create_frame_pipeline_manager_state(worker_reaper_handle);
        let quic_endpoint = QuicEndpoint::new()
            .await
            .context("Failed to create the QUIC endpoint")?;
        let local_address = quic_endpoint.local_address()?;
        let requester_name = format!("{} ({})", config.app_name, local_address.port());

        info!(
            event = "listener_started",
            %local_address,
            "Connection listener started"
        );

        let mut app = App::new(
            config,
            screen_layout_manager_state,
            screen_capture_manager_state,
            frame_pipeline_manager_state,
            quic_endpoint.clone(),
            worker_reaper,
        );
        app.run().await?;

        Ok(Self {
            model: ApplicationModel::new(app, requester_name),
            view,
            messages,
            closing: false,
            finished: false,
            local_port: local_address.port(),
            listener_online: true,
            active_section: WorkspaceSection::default(),
            status_message: String::new(),
            direct_connection: DirectConnectionState::default(),
            screen_stream: ScreenStreamState::default(),
            video_decoder: None,
            pending_screen_stream_starts: HashSet::new(),
            pending_host_screen_stream_stops: HashSet::new(),
            _connection_listener: compio::runtime::spawn(receive_connection_requests(
                quic_endpoint,
                sender.clone(),
            )),
            _logger_guard: logger_guard,
        })
    }

    async fn run(view: ViewPublisher, intents: flume::Receiver<GuiIntent>) -> eros::Result<()> {
        let messages = UnsyncQueue::default();
        let sender = MessageSender {
            messages: messages.clone(),
        };
        let mut application = Self::new(view, messages, &sender).await?;
        application.publish_view_state()?;

        while !application.finished {
            let message = application.next_message(&intents).await;
            let changed = application.update(message, &sender).await?;
            if changed {
                application.publish_view_state()?;
            }
        }

        application.view.quit()?;
        Ok(())
    }

    async fn next_message(&self, intents: &flume::Receiver<GuiIntent>) -> RootMessage {
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
                GuiIntent::OpenRemoteScreen(index) => RootMessage::OpenRemoteScreen(index),
                GuiIntent::DisconnectRemoteSession => RootMessage::DisconnectRemoteSession,
                GuiIntent::StopHostedScreenStream(index) => {
                    RootMessage::StopHostedScreenStream(index)
                }
                GuiIntent::DisconnectDevice(index) => RootMessage::DisconnectDevice(index),
                GuiIntent::RetryConnection => RootMessage::ResetDirectConnection,
                GuiIntent::StopScreenStream => RootMessage::StopCurrentScreenStream,
                GuiIntent::VideoFramePresented {
                    session_id,
                    screen_id,
                } => RootMessage::VideoFramePresented(session_id, screen_id),
                GuiIntent::VideoRendererFailed(error) => RootMessage::VideoRendererFailed(error),
                GuiIntent::Close => RootMessage::Close,
            },
            Either::Right((Err(_), _)) => RootMessage::Close,
        }
    }

    fn publish_view_state(&self) -> eros::Result<()> {
        self.view.publish(self.view_state())
    }

    fn view_state(&self) -> ViewState {
        let connection_requests = self
            .model
            .pending_connection_requests
            .iter()
            .map(|request| ConnectionRequestView {
                name: request.request().requester_name.clone(),
                address: request.remote_address().to_string(),
            })
            .collect::<Vec<_>>();

        let connected_devices = self
            .host_session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let session = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)?;
                let streaming = !session.screen_streams.is_empty();
                Some(ConnectedDeviceView {
                    name: session
                        .peer_name
                        .clone()
                        .unwrap_or_else(|| "Rabbit".to_string()),
                    address: session.key.peer_address().to_string(),
                    status: if streaming {
                        "Streaming".to_string()
                    } else {
                        "Connected".to_string()
                    },
                })
            })
            .collect::<Vec<_>>();

        let hosted_screen_streams = self
            .hosted_screen_stream_entries()
            .into_iter()
            .filter_map(|(session_id, screen_id)| {
                let session = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)?;
                let screen = self.model.app.screen(&screen_id)?;
                Some(HostedScreenStreamView {
                    device_name: session
                        .peer_name
                        .clone()
                        .unwrap_or_else(|| session.key.peer_address().to_string()),
                    screen_name: screen.name.clone(),
                })
            })
            .collect::<Vec<_>>();

        let remote_screens = self
            .model
            .remote_screen_entries
            .iter()
            .filter_map(|(session_id, screen_id)| {
                self.model
                    .remote_screens
                    .get(session_id)?
                    .iter()
                    .find(|screen| screen.id == *screen_id)
                    .map(|screen| RemoteScreenView {
                        name: format!("Session {} · {}", session_id.0, screen.name),
                        resolution: format!(
                            "{} × {}",
                            screen.resolution.width, screen.resolution.height
                        ),
                    })
            })
            .collect::<Vec<_>>();
        let (page, page_title, page_subtitle, status_text, stream_title, stream_resolution) =
            match self.active_section {
                WorkspaceSection::ThisDevice if !connection_requests.is_empty() => (
                    ViewPage::Requests,
                    "Connection requests".to_string(),
                    "Devices are requesting access to this Rabbit instance".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                WorkspaceSection::ThisDevice => (
                    ViewPage::Connected,
                    "This device".to_string(),
                    if connected_devices.is_empty() {
                        "Waiting for clients to connect".to_string()
                    } else {
                        "Clients currently accessing this Rabbit instance".to_string()
                    },
                    self.status_message.clone(),
                    String::new(),
                    String::new(),
                ),
                WorkspaceSection::RemoteDevices => match &self.screen_stream {
                    ScreenStreamState::Requesting(target) => (
                        ViewPage::StreamRequest,
                        "Requesting screen stream...".to_string(),
                        format!(
                            "Requesting {} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Waiting for the remote device to configure the stream".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::WaitingForVideo(target) => (
                        ViewPage::StreamRequest,
                        "Starting screen stream...".to_string(),
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Waiting for the first video frame".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Streaming(target) => (
                        ViewPage::Streaming,
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        "Connected to the remote screen".to_string(),
                        "Receiving video frames".to_string(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Failed { target, message } => (
                        ViewPage::StreamError,
                        "Screen stream failed".to_string(),
                        format!(
                            "{} ({} × {})",
                            target.screen_name, target.frame_size.width, target.frame_size.height
                        ),
                        message.clone(),
                        target.screen_name.clone(),
                        format!("{} × {}", target.frame_size.width, target.frame_size.height),
                    ),
                    ScreenStreamState::Idle => match &self.direct_connection {
                        DirectConnectionState::Connecting { target } => (
                            ViewPage::Connecting,
                            "Connecting...".to_string(),
                            format!("Connecting to {target}"),
                            "Waiting for the remote device to respond".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Rejected { target } => (
                            ViewPage::ConnectionError,
                            "Connection rejected".to_string(),
                            format!("Unable to connect to {target}"),
                            "The remote device rejected the connection request".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::SelfRejected { target } => (
                            ViewPage::ConnectionError,
                            "Cannot connect to this device".to_string(),
                            format!("{target} is this Rabbit instance"),
                            "Enter the address of another Rabbit instance".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Failed { target, message } => (
                            ViewPage::ConnectionError,
                            "Connection failed".to_string(),
                            format!("Unable to connect to {target}"),
                            message.clone(),
                            String::new(),
                            String::new(),
                        ),
                        _ if !remote_screens.is_empty() => (
                            ViewPage::Connected,
                            if let DirectConnectionState::Connected { peer } =
                                &self.direct_connection
                            {
                                format!("Connected to {peer}")
                            } else {
                                "Remote devices".to_string()
                            },
                            "Select a remote screen to open".to_string(),
                            self.status_message.clone(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Connected { peer } => (
                            ViewPage::Connecting,
                            format!("Connected to {peer}"),
                            "Loading the remote screen list".to_string(),
                            "Connection established".to_string(),
                            String::new(),
                            String::new(),
                        ),
                        DirectConnectionState::Idle => (
                            ViewPage::Connect,
                            "Connect to a device".to_string(),
                            "Enter the server IP address or IP:port".to_string(),
                            self.status_message.clone(),
                            String::new(),
                            String::new(),
                        ),
                    },
                },
            };

        let (connection_requests, connected_devices, hosted_screen_streams, remote_screens) =
            match self.active_section {
                WorkspaceSection::RemoteDevices => {
                    (Vec::new(), Vec::new(), Vec::new(), remote_screens)
                }
                WorkspaceSection::ThisDevice => (
                    connection_requests,
                    connected_devices,
                    hosted_screen_streams,
                    Vec::new(),
                ),
            };

        ViewState {
            section: self.active_section,
            page,
            page_title,
            page_subtitle,
            status_text,
            local_port: self.local_port.to_string(),
            local_server_online: self.listener_online,
            stream_title,
            stream_resolution,
            connection_requests,
            connected_devices,
            hosted_screen_streams,
            remote_screens,
        }
    }

    async fn update(&mut self, message: RootMessage, sender: &MessageSender) -> eros::Result<bool> {
        if self.closing && !matches!(&message, RootMessage::ShutdownFinished) {
            return Ok(false);
        }

        match message {
            RootMessage::SelectSection(section) => {
                if self.active_section == section {
                    return Ok(false);
                }
                self.active_section = section;
                Ok(true)
            }
            RootMessage::ResetDirectConnection => {
                self.direct_connection.reset();
                self.status_message.clear();
                Ok(true)
            }
            RootMessage::StopCurrentScreenStream => {
                let Some((session_id, screen_id)) = self.screen_stream.active_screen() else {
                    return Ok(false);
                };
                self.stop_video_decoder()?;
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)
                else {
                    self.screen_stream.reset();
                    self.model.selected_remote_screen = None;
                    return Ok(true);
                };
                let session = Rc::clone(&session.send);
                let stop_sender = sender.clone();

                compio::runtime::spawn(async move {
                    let result = session.stop_screen_stream(screen_id).await;
                    stop_sender.post(RootMessage::ScreenStreamStopFinished {
                        session_id,
                        screen_id,
                        result,
                    });
                })
                .detach();

                Ok(false)
            }
            RootMessage::DisconnectRemoteSession => {
                let Some(session_id) = self.controller_session_id() else {
                    return Ok(false);
                };
                self.disconnect_session(session_id)
            }
            RootMessage::DisconnectDevice(index) => {
                let Some(session_id) = self.host_session_ids().get(index).copied() else {
                    return Ok(false);
                };
                self.disconnect_session(session_id)
            }
            RootMessage::StopHostedScreenStream(index) => {
                let Some((session_id, screen_id)) =
                    self.hosted_screen_stream_entries().get(index).copied()
                else {
                    return Ok(false);
                };
                if !self
                    .pending_host_screen_stream_stops
                    .insert((session_id, screen_id))
                {
                    return Ok(false);
                }
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == session_id)
                else {
                    self.pending_host_screen_stream_stops
                        .remove(&(session_id, screen_id));
                    return Ok(false);
                };
                let session_send = Rc::clone(&session.send);
                let stop_sender = sender.clone();

                compio::runtime::spawn(async move {
                    let result = session_send.stop_screen_stream(screen_id).await;
                    stop_sender.post(RootMessage::HostScreenStreamStopFinished {
                        session_id,
                        screen_id,
                        result,
                    });
                })
                .detach();
                self.set_connection_status(format!(
                    "Stopping Session {} screen {} stream",
                    session_id.0, screen_id.0
                ));
                Ok(true)
            }
            RootMessage::Close => {
                self.closing = true;
                self.stop_video_decoder()?;
                let tasks = self.model.begin_screen_stream_shutdown();
                let sessions = self
                    .model
                    .sessions
                    .iter()
                    .map(|session| Rc::clone(&session.send))
                    .collect::<Vec<_>>();
                let shutdown_sender = sender.clone();

                info!(
                    event = "application_shutdown_started",
                    screen_stream_count = tasks.len(),
                    "Application shutdown started"
                );
                compio::runtime::spawn(async move {
                    for task in tasks {
                        if let Err(error) = task.await {
                            error!(
                                error = ?error,
                                "Screen stream task failed during application shutdown"
                            );
                        }
                    }

                    for session in sessions {
                        session.close().await;
                    }

                    shutdown_sender.post(RootMessage::ShutdownFinished);
                })
                .detach();

                Ok(false)
            }
            RootMessage::ShutdownFinished => {
                info!(
                    event = "application_shutdown_finished",
                    "Application shutdown finished"
                );
                self.finished = true;
                Ok(false)
            }
            RootMessage::ConnectDirect(input) => {
                if self.direct_connection.is_connecting() {
                    self.set_connection_status("Connection already in progress");
                    return Ok(true);
                }

                let (remote_ip, remote_port) = match Self::parse_direct_target(&input) {
                    Ok(target) => target,
                    Err(error) => {
                        self.set_connection_status(format!("Invalid address: {error}"));
                        return Ok(true);
                    }
                };
                if self.model.has_controller_session(remote_ip, remote_port) {
                    self.set_connection_status("Session already connected");
                    return Ok(true);
                }
                let target = DirectTarget::new(remote_ip, remote_port);
                if !self.direct_connection.begin(target) {
                    self.set_connection_status("Connection already in progress");
                    return Ok(true);
                }
                let endpoint: &QuicEndpoint = self.model.app.as_ref();
                let endpoint = endpoint.clone();
                let request = ConnectionRequest {
                    requester_name: self.model.requester_name.clone(),
                };
                let connection_sender = sender.clone();

                info!(
                    event = "direct_connection_started",
                    %remote_ip,
                    ?remote_port,
                    "Direct connection started"
                );
                compio::runtime::spawn(async move {
                    let result =
                        connect_transport(&endpoint, remote_ip, remote_port, request).await;
                    connection_sender.post(RootMessage::DirectConnectionFinished(result));
                })
                .detach();

                Ok(true)
            }
            RootMessage::DirectConnectionFinished(result) => {
                match result {
                    Ok(DirectConnectionOutcome::Connected(transport)) => {
                        let peer_address = transport.remote_address();
                        self.direct_connection
                            .complete(DirectConnectionCompletion::Connected(peer_address));
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Controller, transport);
                        let (send, recv) = session.split();

                        self.start_session(peer_address, None, send, recv, sender);
                    }
                    Ok(DirectConnectionOutcome::Rejected) => {
                        self.direct_connection
                            .complete(DirectConnectionCompletion::Rejected);
                    }
                    Ok(DirectConnectionOutcome::SelfConnection) => {
                        self.direct_connection
                            .complete(DirectConnectionCompletion::SelfRejected);
                    }
                    Err(error) => {
                        self.direct_connection
                            .complete(DirectConnectionCompletion::Failed(error.to_string()));
                    }
                }
                Ok(true)
            }
            RootMessage::ConnectionRequest(request) => {
                self.model.pending_connection_requests.push(request);
                Ok(true)
            }
            RootMessage::AcceptConnectionRequest(index) => {
                let Some(request) = self.take_connection_request(index) else {
                    return Ok(false);
                };
                let peer_name = request.request().requester_name.clone();
                let approval_sender = sender.clone();

                info!(
                    event = "connection_request_decided",
                    remote_address = %request.remote_address(),
                    requester_name = %request.request().requester_name,
                    decision = "accepted",
                    "Connection request decided"
                );
                compio::runtime::spawn(async move {
                    approval_sender.post(RootMessage::ConnectionAccepted {
                        peer_name,
                        result: request.accept().await,
                    });
                })
                .detach();

                Ok(true)
            }
            RootMessage::RejectConnectionRequest(index) => {
                let Some(request) = self.take_connection_request(index) else {
                    return Ok(false);
                };
                let approval_sender = sender.clone();

                info!(
                    event = "connection_request_decided",
                    remote_address = %request.remote_address(),
                    requester_name = %request.request().requester_name,
                    decision = "rejected",
                    "Connection request decided"
                );
                compio::runtime::spawn(async move {
                    approval_sender.post(RootMessage::ConnectionRejected(request.reject().await));
                })
                .detach();

                Ok(true)
            }
            RootMessage::ConnectionAccepted { peer_name, result } => {
                match result {
                    Ok(transport) => {
                        let peer_address = transport.remote_address();
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Host, transport);
                        let (send, recv) = session.split();
                        let screen_list = OutgoingScreenList::try_from(self.model.app.screens())?;
                        let session = PendingHostSession {
                            peer_address,
                            peer_name,
                            send,
                            recv,
                        };
                        let screen_list_sender = sender.clone();

                        compio::runtime::spawn(async move {
                            let result = session.send.send_screen_list(screen_list).await;
                            screen_list_sender
                                .post(RootMessage::InitialScreenListFinished { session, result });
                        })
                        .detach();
                    }
                    Err(error) => {
                        error!(error = ?error, "Failed to accept a QUIC connection request")
                    }
                }

                Ok(false)
            }
            RootMessage::InitialScreenListFinished { session, result } => {
                let changed = match result {
                    Ok(()) => self.start_session(
                        session.peer_address,
                        Some(session.peer_name),
                        session.send,
                        session.recv,
                        sender,
                    ),
                    Err(error) => {
                        error!(error = ?error, "Failed to send the initial screen list");
                        false
                    }
                };

                Ok(changed)
            }
            RootMessage::ConnectionRejected(result) => {
                if let Err(error) = result {
                    error!(error = ?error, "Failed to reject a QUIC connection request");
                }

                Ok(false)
            }
            RootMessage::ConnectionRequestFailed(error) => {
                warn!(error = ?error, "Failed to receive a QUIC connection request");
                Ok(false)
            }
            RootMessage::ConnectionListenerFailed(error) => {
                error!(error = ?error, "QUIC connection listener stopped");
                self.listener_online = false;
                self.set_connection_status("The local connection listener stopped");
                Ok(true)
            }
            RootMessage::SessionMessageReceived(id, message) => {
                match message {
                    SessionMessage::Control(ControlMessage::ScreenList(screens)) => {
                        self.set_connection_status(format!(
                            "Session {} reported {} screens",
                            id.0,
                            screens.len()
                        ));
                        self.model.remote_screens.insert(id, screens);
                        self.refresh_remote_screen_list();
                    }
                    SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
                        let (configured, streams) = self.configure_preserved_screens(request);
                        let Some(session) = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == id)
                        else {
                            warn!(
                                session_id = id.0,
                                "Session closed before screen stream results could be sent"
                            );
                            return Ok(false);
                        };
                        let session_send = Rc::clone(&session.send);
                        self.pending_screen_stream_starts
                            .extend(streams.iter().map(|(screen_id, _)| (id, *screen_id)));
                        let configuration_sender = sender.clone();

                        compio::runtime::spawn(async move {
                            let result = session_send
                                .send_screen_streams_configured(configured)
                                .await;
                            configuration_sender.post(
                                RootMessage::ScreenStreamConfigurationFinished {
                                    session_id: id,
                                    streams,
                                    result,
                                },
                            );
                        })
                        .detach();
                    }
                    SessionMessage::Control(ControlMessage::ScreenStreamsConfigured(
                        configured,
                    )) => {
                        self.screen_stream.apply_configuration(&configured);
                        let configured_count = configured
                            .outcomes
                            .iter()
                            .filter(|outcome| {
                                matches!(&outcome.status, ScreenResolutionStatus::Configured(_))
                            })
                            .count();
                        let failed_count = configured.outcomes.len() - configured_count;

                        self.set_connection_status(format!(
                            "Session {} request {}: {} configured, {} failed",
                            id.0, configured.request_id.0, configured_count, failed_count
                        ));
                        self.model.screen_stream_results.insert(id, configured);
                    }
                    SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
                        let role = self
                            .model
                            .sessions
                            .iter()
                            .find(|session| session.send.id() == id)
                            .map(|session| session.key.role());
                        match role {
                            Some(SessionRole::Host) => {
                                self.pending_screen_stream_starts
                                    .remove(&(id, stop.screen_id));
                                if let Some(session) = self
                                    .model
                                    .sessions
                                    .iter_mut()
                                    .find(|session| session.send.id() == id)
                                {
                                    session.screen_streams.remove(&stop.screen_id);
                                }
                            }
                            Some(SessionRole::Controller) => {
                                if self.screen_stream.active_screen() == Some((id, stop.screen_id))
                                {
                                    self.stop_video_decoder()?;
                                    self.screen_stream.reset();
                                    self.model.selected_remote_screen = None;
                                    self.set_connection_status(format!(
                                        "The remote device stopped screen {} stream",
                                        stop.screen_id.0
                                    ));
                                }
                            }
                            None => return Ok(false),
                        }
                        info!(
                            event = "screen_stream_stop_received",
                            session_id = id.0,
                            screen_id = stop.screen_id.0,
                            "Screen stream stop received"
                        );
                    }
                    SessionMessage::Video(_) => {
                        eros::bail!("Video frame bypassed the latest-frame session queue")
                    }
                }

                Ok(true)
            }
            RootMessage::VideoFrameAvailable(id, screen_id) => {
                let Some(session) = self
                    .model
                    .sessions
                    .iter()
                    .find(|session| session.send.id() == id)
                else {
                    return Ok(false);
                };
                let Some(video) = session.received_video_frames.take(&screen_id) else {
                    return Ok(false);
                };
                let payload_size = video.packets.iter().map(bytes::Bytes::len).sum::<usize>();

                trace!(
                    session_id = id.0,
                    screen_id = video.screen_id.0,
                    packet_count = video.packets.len(),
                    payload_size,
                    "Received latest complete video RTP frame"
                );

                if self.screen_stream.active_screen() != Some((id, screen_id)) {
                    return Ok(false);
                }
                self.start_video_decoder(id, screen_id, sender)?;
                let decoder = self
                    .video_decoder
                    .as_ref()
                    .context("Video decoder was not retained after startup")?;
                decoder.publish(video).with_context(|| {
                    format!("Failed to queue screen {} for decoding", screen_id.0)
                })?;
                Ok(false)
            }
            RootMessage::VideoFramePresented(id, screen_id) => {
                Ok(self.screen_stream.receive_video(id, screen_id))
            }
            RootMessage::VideoDecoderFinished(id, screen_id, result) => {
                if !self
                    .video_decoder
                    .as_ref()
                    .is_some_and(|decoder| decoder.matches(id, screen_id))
                {
                    return Ok(false);
                }
                self.video_decoder = None;
                self.view.clear_video()?;
                match result {
                    Ok(()) => {
                        if self.screen_stream.fail(
                            id,
                            screen_id,
                            "The hardware video decoder ended unexpectedly".to_string(),
                        ) {
                            self.set_connection_status(format!(
                                "Screen {} decoder ended unexpectedly",
                                screen_id.0
                            ));
                            return Ok(true);
                        }
                    }
                    Err(error) => {
                        error!(
                            event = "video_decoder_failed",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Hardware video decoder failed"
                        );
                        if self.screen_stream.fail(
                            id,
                            screen_id,
                            format!("Hardware video decoding failed: {error}"),
                        ) {
                            self.set_connection_status(format!(
                                "Screen {} decoder failed: {error}",
                                screen_id.0
                            ));
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            RootMessage::VideoRendererFailed(message) => {
                let active = self.screen_stream.active_screen();
                self.video_decoder = None;
                if let Some((session_id, screen_id)) = active {
                    error!(
                        event = "video_renderer_failed",
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = %message,
                        "DMA-BUF video renderer failed"
                    );
                    if self.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("GPU video rendering failed: {message}"),
                    ) {
                        self.set_connection_status(format!(
                            "Screen {} renderer failed: {message}",
                            screen_id.0
                        ));
                        return Ok(true);
                    }
                }
                eros::bail!("Slint DMA-BUF video renderer failed: {}", message)
            }
            RootMessage::ScreenStreamConfigurationFinished {
                session_id,
                streams,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        error = ?error,
                        "Failed to send screen stream results"
                    );
                    self.remove_session(session_id);
                    return Ok(true);
                }

                if !self
                    .model
                    .sessions
                    .iter()
                    .any(|session| session.send.id() == session_id)
                {
                    return Ok(false);
                }

                let mut changed = false;
                for (screen_id, parameters) in streams {
                    if !self
                        .pending_screen_stream_starts
                        .remove(&(session_id, screen_id))
                    {
                        continue;
                    }
                    if let Err(error) =
                        self.replace_screen_stream(session_id, screen_id, parameters, sender)
                    {
                        error!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Failed to start screen stream"
                        );
                    } else {
                        changed = true;
                    }
                }

                Ok(changed)
            }
            RootMessage::SessionClosed(id) => {
                self.stop_session_video_decoder(id)?;
                self.screen_stream
                    .fail_session(id, "The remote session closed".to_string());
                self.remove_session(id);
                info!(
                    event = "session_closed",
                    session_id = id.0,
                    "Session closed"
                );
                self.set_connection_status(format!("Session {} closed", id.0));
                Ok(true)
            }
            RootMessage::SessionFailed(id, error) => {
                self.stop_session_video_decoder(id)?;
                self.screen_stream
                    .fail_session(id, format!("The remote session failed: {error}"));
                self.remove_session(id);
                error!(session_id = id.0, error = ?error, "Session receive loop failed");
                self.set_connection_status(format!("Session {} failed: {error}", id.0));
                Ok(true)
            }
            RootMessage::ScreenStreamFinished(id, screen_id, stream_id, result) => {
                let Some(session) = self
                    .model
                    .sessions
                    .iter_mut()
                    .find(|session| session.send.id() == id)
                else {
                    return Ok(false);
                };
                let is_current = session
                    .screen_streams
                    .get(&screen_id)
                    .is_some_and(|stream| stream.id == stream_id);

                if !is_current {
                    return Ok(false);
                }

                let session_closed_normally = session.send.is_closed_normally();
                session.screen_streams.remove(&screen_id);

                match result {
                    Ok(()) => info!(
                        event = "screen_stream_finished",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        "Screen stream finished"
                    ),
                    Err(_) if session_closed_normally => info!(
                        event = "screen_stream_finished",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        "Screen stream finished during normal Session close"
                    ),
                    Err(error) => {
                        error!(
                            event = "screen_stream_failed",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Screen stream failed"
                        );
                        self.set_connection_status(format!(
                            "Session {} screen {} failed: {error}",
                            id.0, screen_id.0
                        ));
                    }
                }

                Ok(true)
            }
            RootMessage::OpenRemoteScreen(selected_index) => {
                let previous = self.model.selected_remote_screen;
                let selected = self
                    .model
                    .remote_screen_entries
                    .get(selected_index)
                    .copied();

                if selected == previous {
                    return Ok(false);
                }
                self.stop_video_decoder()?;
                self.model.selected_remote_screen = selected;

                if let Some((session_id, screen_id)) = selected {
                    let Some((screen_name, frame_size)) = self
                        .model
                        .remote_screens
                        .get(&session_id)
                        .and_then(|screens| {
                            screens
                                .iter()
                                .find(|screen| screen.id == screen_id)
                                .map(|screen| (screen.name.clone(), screen.resolution))
                        })
                    else {
                        warn!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            "Selected remote screen is no longer available"
                        );
                        return Ok(false);
                    };
                    let Some(session) = self
                        .model
                        .sessions
                        .iter()
                        .find(|session| session.send.id() == session_id)
                    else {
                        warn!(
                            session_id = session_id.0,
                            "Session closed before screen stream could be requested"
                        );
                        return Ok(false);
                    };
                    let session_send = Rc::clone(&session.send);
                    let request_id = self.model.next_screen_stream_request_id()?;
                    self.screen_stream.begin(ScreenStreamTarget {
                        request_id,
                        session_id,
                        screen_id,
                        screen_name,
                        frame_size,
                    });
                    let request = SetScreenStreams {
                        request_id,
                        desired_streams: vec![ScreenStreamRequest {
                            screen_id,
                            remote_display: RemoteDisplayMode::Preserve,
                            frame_size,
                        }],
                    };

                    let request_sender = sender.clone();
                    compio::runtime::spawn(async move {
                        let result = session_send.send_screen_streams_request(request).await;
                        request_sender.post(RootMessage::ScreenStreamRequestFinished {
                            request_id,
                            session_id,
                            screen_id,
                            frame_size,
                            result,
                        });
                    })
                    .detach();
                }

                Ok(true)
            }
            RootMessage::ScreenStreamRequestFinished {
                request_id,
                session_id,
                screen_id,
                frame_size,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to request screen stream"
                    );
                    if self.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("Failed to request screen stream: {error}"),
                    ) {
                        self.remove_session(session_id);
                        return Ok(true);
                    }
                    self.remove_session(session_id);
                } else {
                    if !self
                        .model
                        .sessions
                        .iter()
                        .any(|session| session.send.id() == session_id)
                    {
                        return Ok(false);
                    }
                    trace!(
                        request_id = request_id.0,
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        width = frame_size.width,
                        height = frame_size.height,
                        "Screen stream request sent"
                    );
                }

                Ok(true)
            }
            RootMessage::ScreenStreamStopFinished {
                session_id,
                screen_id,
                result,
            } => {
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to stop screen stream"
                    );
                    self.screen_stream.fail(
                        session_id,
                        screen_id,
                        format!("Failed to stop screen stream: {error}"),
                    );
                    return Ok(true);
                }

                self.screen_stream.reset();
                self.stop_video_decoder()?;
                self.model.selected_remote_screen = None;
                self.set_connection_status(format!(
                    "Stopped screen {} stream; Session {} remains connected",
                    screen_id.0, session_id.0
                ));
                Ok(true)
            }
            RootMessage::HostScreenStreamStopFinished {
                session_id,
                screen_id,
                result,
            } => {
                self.pending_host_screen_stream_stops
                    .remove(&(session_id, screen_id));
                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to notify the remote device that its screen stream was stopped"
                    );
                    self.set_connection_status(format!(
                        "Failed to stop Session {} screen {}: {error}",
                        session_id.0, screen_id.0
                    ));
                    return Ok(true);
                }

                let Some(session) = self
                    .model
                    .sessions
                    .iter_mut()
                    .find(|session| session.send.id() == session_id)
                else {
                    return Ok(false);
                };
                session.screen_streams.remove(&screen_id);
                info!(
                    event = "host_screen_stream_stopped",
                    session_id = session_id.0,
                    screen_id = screen_id.0,
                    "Host stopped screen stream"
                );
                self.set_connection_status(format!(
                    "Stopped Session {} screen {} stream",
                    session_id.0, screen_id.0
                ));
                Ok(true)
            }
        }
    }
}

async fn receive_connection_requests(endpoint: QuicEndpoint, sender: MessageSender) {
    loop {
        let connection = match endpoint.accept_connection().await {
            Ok(Some(connection)) => connection,
            Ok(None) => return,
            Err(error) => {
                sender.post(RootMessage::ConnectionListenerFailed(error));
                return;
            }
        };

        match receive_request(connection).await {
            Ok(Some(request)) => sender.post(RootMessage::ConnectionRequest(request)),
            Ok(None) => {}
            Err(error) => sender.post(RootMessage::ConnectionRequestFailed(error)),
        }
    }
}

async fn receive_session<R>(
    mut session: SessionRecv<R>,
    received_video_frames: LatestVideoFrames,
    sender: MessageSender,
) where
    R: TransportRecv,
{
    let id = session.id();

    loop {
        match session.recv().await {
            Ok(Some(SessionMessage::Video(frame))) => {
                let screen_id = frame.screen_id;
                let first_pending_frame = received_video_frames.publish(frame);
                if first_pending_frame {
                    sender.post(RootMessage::VideoFrameAvailable(id, screen_id));
                }
            }
            Ok(Some(message)) => sender.post(RootMessage::SessionMessageReceived(id, message)),
            Ok(None) => {
                sender.post(RootMessage::SessionClosed(id));
                return;
            }
            Err(error) => {
                sender.post(RootMessage::SessionFailed(id, error));
                return;
            }
        }
    }
}
