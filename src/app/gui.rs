use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    rc::Rc,
};

use eros::Context;
use tracing::{error, info, trace, warn};
use winio::prelude::*;

use crate::{
    app::{App, LoggerGuard, config::Config, init_logging, screen_stream::run_host_screen_stream},
    infra::{
        GbmFramePipelineManagerState, KmsScreenCaptureManagerState, NiriScreenLayoutManagerState,
        PendingQuicConnectionRequest, QuicEndpoint, QuicTransport, QuicTransportSend,
        connect_transport, create_frame_pipeline_manager_state,
        create_screen_capture_manager_state, create_screen_layout_manager_state, receive_request,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        connection_request::ConnectionRequest,
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        screen_configuration::{
            RemoteDisplayMode, ResolutionResult, ScreenResolutionOutcome, ScreenResolutionStatus,
            ScreenStreamRequest, ScreenStreamRequestId, ScreenStreamsConfigured, SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{Session, SessionId, SessionMessage, SessionRecv, SessionRole, SessionSend},
        session_control::{ControlMessage, ScreenInfo},
        transport::TransportRecv,
    },
};

struct RunningSession {
    send: Rc<SessionSend<QuicTransportSend>>,
    screen_streams: HashMap<ScreenId, RunningScreenStream>,
    _receiver: compio::runtime::JoinHandle<()>,
}

struct RunningScreenStream {
    id: u64,
    cancellation: UnsyncQueue<()>,
    task: Option<compio::runtime::JoinHandle<()>>,
}

impl Drop for RunningScreenStream {
    fn drop(&mut self) {
        self.cancellation.push(());

        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

pub(crate) struct RootComponent {
    _app: App<
        NiriScreenLayoutManagerState,
        KmsScreenCaptureManagerState,
        GbmFramePipelineManagerState,
    >,
    window: Child<Window>,
    direct_address_input: Child<Edit>,
    connect_button: Child<Button>,
    connection_status: Child<Label>,
    remote_screen_title: Child<Label>,
    remote_screen_list: Child<ListBox>,
    requester_name: String,
    connection_request_title: Child<Label>,
    connection_request_list: Child<ListBox>,
    accept_connection_button: Child<Button>,
    reject_connection_button: Child<Button>,
    pending_connection_requests: Vec<PendingQuicConnectionRequest>,
    sessions: Vec<RunningSession>,
    remote_screens: HashMap<SessionId, Vec<ScreenInfo>>,
    remote_screen_entries: Vec<(SessionId, ScreenId)>,
    selected_remote_screen: Option<(SessionId, ScreenId)>,
    screen_stream_results: HashMap<SessionId, ScreenStreamsConfigured>,
    next_session_id: u32,
    next_screen_stream_id: u64,
    next_screen_stream_request_id: u32,
    _connection_listener: compio::runtime::JoinHandle<()>,
    _logger_guard: LoggerGuard,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
    ConnectDirect,
    DirectConnectionFinished(eros::Result<Option<QuicTransport>>),
    ConnectionRequest(PendingQuicConnectionRequest),
    ConnectionRequestSelectionChanged,
    AcceptSelectedConnection,
    RejectSelectedConnection,
    ConnectionAccepted(eros::Result<QuicTransport>),
    ConnectionRejected(eros::Result<()>),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
    SessionMessageReceived(SessionId, SessionMessage),
    SessionClosed(SessionId),
    SessionFailed(SessionId, eros::ErrorUnion),
    ScreenStreamFinished(SessionId, ScreenId, u64, eros::Result<()>),
    RemoteScreenSelectionChanged,
}

impl RootComponent {
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
                let status = match self._app.screen(&desired_stream.screen_id) {
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
        send: SessionSend<QuicTransportSend>,
        recv: SessionRecv<R>,
        sender: &ComponentSender<Self>,
    ) where
        R: TransportRecv + 'static,
    {
        info!(
            event = "session_started",
            session_id = send.id().0,
            role = ?send.role(),
            "Session started"
        );
        self.sessions.push(RunningSession {
            send: Rc::new(send),
            screen_streams: HashMap::new(),
            _receiver: compio::runtime::spawn(receive_session(recv, sender.clone())),
        });
    }

    fn next_screen_stream_id(&mut self) -> eros::Result<u64> {
        let id = self.next_screen_stream_id;
        self.next_screen_stream_id = self
            .next_screen_stream_id
            .checked_add(1)
            .context("Failed to allocate a screen stream task ID")?;

        Ok(id)
    }

    fn next_screen_stream_request_id(&mut self) -> eros::Result<ScreenStreamRequestId> {
        let id = ScreenStreamRequestId(self.next_screen_stream_request_id);
        self.next_screen_stream_request_id = self
            .next_screen_stream_request_id
            .checked_add(1)
            .context("Failed to allocate a screen stream request ID")?;

        Ok(id)
    }

    fn replace_screen_stream(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
        sender: &ComponentSender<Self>,
    ) -> eros::Result<()> {
        let frames = FramePipelineManager::subscribe(&mut self._app, &screen_id, parameters)?;
        let stream_id = self.next_screen_stream_id()?;
        let Some(session) = self
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
            let result =
                run_host_screen_stream(frames, screen_id, session_send, task_cancellation).await;
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

    fn remove_session(&mut self, id: SessionId) -> eros::Result<()> {
        self.sessions.retain(|session| session.send.id() != id);
        self.remote_screens.remove(&id);
        self.screen_stream_results.remove(&id);
        self.refresh_remote_screen_list()
    }

    fn refresh_remote_screen_list(&mut self) -> eros::Result<()> {
        let mut entries = self
            .remote_screens
            .iter()
            .flat_map(|(session_id, screens)| {
                screens.iter().map(|screen| {
                    (
                        *session_id,
                        screen.id,
                        format!(
                            "Session {} - {}: {} ({}x{})",
                            session_id.0,
                            screen.id.0,
                            screen.name,
                            screen.resolution.width,
                            screen.resolution.height
                        ),
                    )
                })
            })
            .collect::<Vec<_>>();

        entries.sort_by_key(|(session_id, screen_id, _)| (session_id.0, screen_id.0));
        self.remote_screen_entries.clear();
        self.remote_screen_entries.extend(
            entries
                .iter()
                .map(|(session_id, screen_id, _)| (*session_id, *screen_id)),
        );
        self.selected_remote_screen = None;
        self.remote_screen_list
            .set_items(entries.into_iter().map(|(_, _, entry)| entry))?;
        let visible = !self.remote_screen_list.is_empty()?;
        self.remote_screen_title.set_visible(visible)?;
        self.remote_screen_list.set_visible(visible)?;

        Ok(())
    }

    fn next_session_id(&mut self) -> eros::Result<SessionId> {
        let id = SessionId(self.next_session_id);
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .context("Failed to allocate a Session ID")?;

        Ok(id)
    }

    fn parse_direct_target(&self) -> eros::Result<(IpAddr, Option<u16>)> {
        let input = self.direct_address_input.text()?;
        let input = input.trim();

        if let Ok(address) = input.parse::<SocketAddr>() {
            return Ok((address.ip(), Some(address.port())));
        }

        let ip = input
            .parse::<IpAddr>()
            .with_context(|| format!("Failed to parse direct connection IP {input:?}"))?;

        Ok((ip, None))
    }

    fn set_connection_request_panel_visible(&mut self, visible: bool) -> eros::Result<()> {
        self.connection_request_title.set_visible(visible)?;
        self.connection_request_list.set_visible(visible)?;
        self.accept_connection_button.set_visible(visible)?;
        self.reject_connection_button.set_visible(visible)?;
        Ok(())
    }

    fn selected_connection_request_index(&self) -> eros::Result<Option<usize>> {
        for index in 0..self.pending_connection_requests.len() {
            if self.connection_request_list.is_selected(index)? {
                return Ok(Some(index));
            }
        }

        Ok(None)
    }

    fn refresh_connection_request_list(&mut self, selected: Option<usize>) -> eros::Result<()> {
        self.connection_request_list
            .set_items(self.pending_connection_requests.iter().map(|request| {
                format!(
                    "{} - {}",
                    request.request().requester_name,
                    request.remote_address(),
                )
            }))?;
        let visible = !self.pending_connection_requests.is_empty();
        self.set_connection_request_panel_visible(visible)?;

        if let Some(selected) = selected {
            self.connection_request_list.set_selected(selected, true)?;
        }

        Ok(())
    }

    fn take_selected_connection_request(
        &mut self,
    ) -> eros::Result<Option<PendingQuicConnectionRequest>> {
        let Some(selected) = self.selected_connection_request_index()? else {
            return Ok(None);
        };

        let request = self.pending_connection_requests.remove(selected);
        let next = if self.pending_connection_requests.is_empty() {
            None
        } else {
            Some(selected.min(self.pending_connection_requests.len() - 1))
        };
        self.refresh_connection_request_list(next)?;

        Ok(Some(request))
    }
}

impl Component for RootComponent {
    type Error = eros::ErrorUnion;
    type Event = ();
    type Init<'a> = ();
    type Message = RootMessage;

    async fn init(_init: Self::Init<'_>, sender: &ComponentSender<Self>) -> eros::Result<Self> {
        let config = Config::new()?;
        let logger_guard = init_logging(&config)?;
        let screen_layout_manager_state = create_screen_layout_manager_state()
            .context("Failed to create the screen layout manager state")?;
        let screen_capture_manager_state =
            create_screen_capture_manager_state(config.video.enable_probing);
        let frame_pipeline_manager_state = create_frame_pipeline_manager_state();
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
        );
        app.run().await?;

        let mut window = Child::<Window>::init(()).await?;
        window.set_text(format!("Rabbit - UDP {}", local_address.port()))?;
        window.set_size(Size::new(800.0, 600.0))?;
        let mut direct_address_input = Child::<Edit>::init(&window).await?;
        direct_address_input.set_text("127.0.0.1")?;
        let mut connect_button = Child::<Button>::init(&window).await?;
        connect_button.set_text("Connect")?;
        let mut connection_status = Child::<Label>::init(&window).await?;
        connection_status.set_text(format!("Listening on UDP {}", local_address.port()))?;
        let mut remote_screen_title = Child::<Label>::init(&window).await?;
        remote_screen_title.set_text("Remote screens")?;
        remote_screen_title.set_visible(false)?;
        let mut remote_screen_list = Child::<ListBox>::init(&window).await?;
        remote_screen_list.set_multiple(false)?;
        remote_screen_list.set_visible(false)?;
        let mut connection_request_title = Child::<Label>::init(&window).await?;
        connection_request_title.set_text("Pending connection requests")?;
        connection_request_title.set_visible(false)?;
        let mut connection_request_list = Child::<ListBox>::init(&window).await?;
        connection_request_list.set_multiple(false)?;
        connection_request_list.set_visible(false)?;
        let mut accept_connection_button = Child::<Button>::init(&window).await?;
        accept_connection_button.set_text("Accept")?;
        accept_connection_button.set_visible(false)?;
        let mut reject_connection_button = Child::<Button>::init(&window).await?;
        reject_connection_button.set_text("Reject")?;
        reject_connection_button.set_visible(false)?;
        window.show()?;

        Ok(Self {
            _app: app,
            window,
            direct_address_input,
            connect_button,
            connection_status,
            remote_screen_title,
            remote_screen_list,
            requester_name,
            connection_request_title,
            connection_request_list,
            accept_connection_button,
            reject_connection_button,
            pending_connection_requests: Vec::new(),
            sessions: Vec::new(),
            remote_screens: HashMap::new(),
            remote_screen_entries: Vec::new(),
            selected_remote_screen: None,
            screen_stream_results: HashMap::new(),
            next_session_id: 0,
            next_screen_stream_id: 0,
            next_screen_stream_request_id: 0,
            _connection_listener: compio::runtime::spawn(receive_connection_requests(
                quic_endpoint,
                sender.clone(),
            )),
            _logger_guard: logger_guard,
        })
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        start! {
            sender, default: RootMessage::Noop,
            self.window => {
                WindowEvent::Close => RootMessage::Close,
            },
            self.connect_button => {
                ButtonEvent::Click => RootMessage::ConnectDirect,
            },
            self.connection_request_list => {
                ListBoxEvent::Select => RootMessage::ConnectionRequestSelectionChanged,
            },
            self.remote_screen_list => {
                ListBoxEvent::Select => RootMessage::RemoteScreenSelectionChanged,
            },
            self.accept_connection_button => {
                ButtonEvent::Click => RootMessage::AcceptSelectedConnection,
            },
            self.reject_connection_button => {
                ButtonEvent::Click => RootMessage::RejectSelectedConnection,
            },
        }
    }

    async fn update_children(&mut self) -> eros::Result<bool> {
        let mut changed = self.window.update().await?;
        changed |= self.direct_address_input.update().await?;
        changed |= self.connect_button.update().await?;
        changed |= self.connection_status.update().await?;
        changed |= self.remote_screen_title.update().await?;
        changed |= self.remote_screen_list.update().await?;
        changed |= self.connection_request_title.update().await?;
        changed |= self.connection_request_list.update().await?;
        changed |= self.accept_connection_button.update().await?;
        changed |= self.reject_connection_button.update().await?;
        Ok(changed)
    }

    async fn update(
        &mut self,
        message: Self::Message,
        sender: &ComponentSender<Self>,
    ) -> eros::Result<bool> {
        match message {
            RootMessage::Noop => Ok(false),
            RootMessage::Close => {
                sender.output(());
                Ok(false)
            }
            RootMessage::ConnectDirect => {
                let (remote_ip, remote_port) = match self.parse_direct_target() {
                    Ok(target) => target,
                    Err(error) => {
                        self.connection_status
                            .set_text(format!("Invalid address: {error}"))?;
                        return Ok(true);
                    }
                };
                let endpoint: &QuicEndpoint = self._app.as_ref();
                let endpoint = endpoint.clone();
                let request = ConnectionRequest {
                    requester_name: self.requester_name.clone(),
                };
                let connection_sender = sender.clone();

                info!(
                    event = "direct_connection_started",
                    %remote_ip,
                    ?remote_port,
                    "Direct connection started"
                );
                self.connect_button.set_enabled(false)?;
                self.connection_status.set_text("Connecting...")?;

                compio::runtime::spawn(async move {
                    let result =
                        connect_transport(&endpoint, remote_ip, remote_port, request).await;
                    connection_sender.post(RootMessage::DirectConnectionFinished(result));
                })
                .detach();

                Ok(true)
            }
            RootMessage::DirectConnectionFinished(result) => {
                self.connect_button.set_enabled(true)?;

                match result {
                    Ok(Some(transport)) => {
                        let id = self.next_session_id()?;
                        let session = Session::new(id, SessionRole::Controller, transport);
                        let (send, recv) = session.split();

                        self.start_session(send, recv, sender);
                        self.connection_status.set_text("Connection accepted")?;
                    }
                    Ok(None) => self.connection_status.set_text("Connection rejected")?,
                    Err(error) => self
                        .connection_status
                        .set_text(format!("Connection failed: {error}"))?,
                }

                Ok(true)
            }
            RootMessage::ConnectionRequest(request) => {
                let first_request = self.pending_connection_requests.is_empty();
                let selected = self.selected_connection_request_index()?;

                self.pending_connection_requests.push(request);
                self.refresh_connection_request_list(selected.or(first_request.then_some(0)))?;

                Ok(true)
            }
            RootMessage::ConnectionRequestSelectionChanged => Ok(false),
            RootMessage::AcceptSelectedConnection => {
                let Some(request) = self.take_selected_connection_request()? else {
                    return Ok(false);
                };
                let approval_sender = sender.clone();

                info!(
                    event = "connection_request_decided",
                    remote_address = %request.remote_address(),
                    requester_name = %request.request().requester_name,
                    decision = "accepted",
                    "Connection request decided"
                );
                compio::runtime::spawn(async move {
                    approval_sender.post(RootMessage::ConnectionAccepted(request.accept().await));
                })
                .detach();

                Ok(true)
            }
            RootMessage::RejectSelectedConnection => {
                let Some(request) = self.take_selected_connection_request()? else {
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
            RootMessage::ConnectionAccepted(result) => {
                match result {
                    Ok(transport) => {
                        let id = self.next_session_id()?;
                        let session = Session::new(id, SessionRole::Host, transport);
                        let (send, recv) = session.split();

                        match send.send_screen_list(self._app.screens()).await {
                            Ok(()) => self.start_session(send, recv, sender),
                            Err(error) => {
                                error!(%error, "Failed to send the initial screen list")
                            }
                        }
                    }
                    Err(error) => error!(%error, "Failed to accept a QUIC connection request"),
                }

                Ok(false)
            }
            RootMessage::ConnectionRejected(result) => {
                if let Err(error) = result {
                    error!(%error, "Failed to reject a QUIC connection request");
                }

                Ok(false)
            }
            RootMessage::ConnectionRequestFailed(error) => {
                warn!(%error, "Failed to receive a QUIC connection request");
                Ok(false)
            }
            RootMessage::ConnectionListenerFailed(error) => {
                error!(%error, "QUIC connection listener stopped");
                Ok(false)
            }
            RootMessage::SessionMessageReceived(id, message) => {
                match message {
                    SessionMessage::Control(ControlMessage::ScreenList(screens)) => {
                        self.connection_status.set_text(format!(
                            "Session {} reported {} screens",
                            id.0,
                            screens.len()
                        ))?;
                        self.remote_screens.insert(id, screens);
                        self.refresh_remote_screen_list()?;
                    }
                    SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
                        let (configured, streams) = self.configure_preserved_screens(request);
                        let Some(session) =
                            self.sessions.iter().find(|session| session.send.id() == id)
                        else {
                            warn!(
                                session_id = id.0,
                                "Session closed before screen stream results could be sent"
                            );
                            return Ok(false);
                        };
                        let session_send = Rc::clone(&session.send);

                        if let Err(error) = session_send
                            .send_screen_streams_configured(configured)
                            .await
                        {
                            error!(session_id = id.0, %error, "Failed to send screen stream results");
                            self.remove_session(id)?;
                            return Ok(false);
                        }

                        for (screen_id, parameters) in streams {
                            if let Err(error) =
                                self.replace_screen_stream(id, screen_id, parameters, sender)
                            {
                                error!(
                                    session_id = id.0,
                                    screen_id = screen_id.0,
                                    %error,
                                    "Failed to start screen stream"
                                );
                            }
                        }
                    }
                    SessionMessage::Control(ControlMessage::ScreenStreamsConfigured(
                        configured,
                    )) => {
                        let configured_count = configured
                            .outcomes
                            .iter()
                            .filter(|outcome| {
                                matches!(&outcome.status, ScreenResolutionStatus::Configured(_))
                            })
                            .count();
                        let failed_count = configured.outcomes.len() - configured_count;

                        self.connection_status.set_text(format!(
                            "Session {} request {}: {} configured, {} failed",
                            id.0, configured.request_id.0, configured_count, failed_count
                        ))?;
                        self.screen_stream_results.insert(id, configured);
                    }
                    SessionMessage::Video(video) => trace!(
                        session_id = id.0,
                        screen_id = video.screen_id.0,
                        packet_size = video.payload.len(),
                        "Received video RTP packet"
                    ),
                }

                Ok(true)
            }
            RootMessage::SessionClosed(id) => {
                self.remove_session(id)?;
                info!(
                    event = "session_closed",
                    session_id = id.0,
                    "Session closed"
                );
                self.connection_status
                    .set_text(format!("Session {} closed", id.0))?;
                Ok(true)
            }
            RootMessage::SessionFailed(id, error) => {
                self.remove_session(id)?;
                error!(session_id = id.0, %error, "Session receive loop failed");
                self.connection_status
                    .set_text(format!("Session {} failed: {error}", id.0))?;
                Ok(true)
            }
            RootMessage::ScreenStreamFinished(id, screen_id, stream_id, result) => {
                let Some(session) = self
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

                session.screen_streams.remove(&screen_id);

                match result {
                    Ok(()) => info!(
                        event = "screen_stream_finished",
                        session_id = id.0,
                        screen_id = screen_id.0,
                        "Screen stream finished"
                    ),
                    Err(error) => {
                        error!(
                            event = "screen_stream_failed",
                            session_id = id.0,
                            screen_id = screen_id.0,
                            %error,
                            "Screen stream failed"
                        );
                        self.connection_status.set_text(format!(
                            "Session {} screen {} failed: {error}",
                            id.0, screen_id.0
                        ))?;
                    }
                }

                Ok(true)
            }
            RootMessage::RemoteScreenSelectionChanged => {
                let previous = self.selected_remote_screen;
                let mut selected = None;

                for (index, entry) in self.remote_screen_entries.iter().enumerate() {
                    if self.remote_screen_list.is_selected(index)? {
                        selected = Some(*entry);
                        break;
                    }
                }

                self.selected_remote_screen = selected;

                if selected == previous {
                    return Ok(false);
                }

                if let Some((session_id, screen_id)) = selected {
                    let Some(frame_size) =
                        self.remote_screens.get(&session_id).and_then(|screens| {
                            screens
                                .iter()
                                .find(|screen| screen.id == screen_id)
                                .map(|screen| screen.resolution)
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
                    let request_id = self.next_screen_stream_request_id()?;
                    let request = SetScreenStreams {
                        request_id,
                        desired_streams: vec![ScreenStreamRequest {
                            screen_id,
                            remote_display: RemoteDisplayMode::Preserve,
                            frame_size,
                        }],
                    };

                    if let Err(error) = session_send.send_screen_streams_request(request).await {
                        error!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            %error,
                            "Failed to request screen stream"
                        );
                        self.remove_session(session_id)?;
                        self.connection_status.set_text(format!(
                            "Session {} screen request failed: {error}",
                            session_id.0
                        ))?;
                    } else {
                        self.connection_status.set_text(format!(
                            "Requested session {} screen {} at {}x{}",
                            session_id.0, screen_id.0, frame_size.width, frame_size.height
                        ))?;
                    }
                }

                Ok(true)
            }
        }
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> eros::Result<()> {
        let size = self.window.size()?;
        let mut direct_connection = layout! {
            StackPanel::new(Orient::Horizontal),
            self.direct_address_input => { grow: true },
            self.connect_button,
        };
        let mut actions = layout! {
            StackPanel::new(Orient::Horizontal),
            self.reject_connection_button => { grow: true },
            self.accept_connection_button => { grow: true },
        };
        let mut panel = layout! {
            StackPanel::new(Orient::Vertical),
            direct_connection,
            self.connection_status,
            self.remote_screen_title,
            self.remote_screen_list => { grow: true },
            self.connection_request_title,
            self.connection_request_list => { grow: true },
            actions,
        };

        panel.set_size(size)?;
        Ok(())
    }
}

async fn receive_connection_requests(
    endpoint: QuicEndpoint,
    sender: ComponentSender<RootComponent>,
) {
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
            Ok(request) => sender.post(RootMessage::ConnectionRequest(request)),
            Err(error) => sender.post(RootMessage::ConnectionRequestFailed(error)),
        }
    }
}

async fn receive_session<R>(mut session: SessionRecv<R>, sender: ComponentSender<RootComponent>)
where
    R: TransportRecv,
{
    let id = session.id();

    loop {
        match session.recv().await {
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
