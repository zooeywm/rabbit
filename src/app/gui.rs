use std::{
    net::{IpAddr, SocketAddr},
    rc::Rc,
};

use eros::Context;
use tracing::{error, info, trace, warn};
use winio::prelude::*;

use crate::app::{
    gui::view::{RootView, RootViewEvent, RootViewInit, RootViewMessage},
    model::{ApplicationModel, LatestVideoFrames, RunningScreenStream, RunningSession},
};

use crate::{
    app::{App, LoggerGuard, config::Config, init_logging, screen_stream::run_host_screen_stream},
    infra::{
        PendingQuicConnectionRequest, QuicEndpoint, QuicTransport, QuicTransportRecv,
        QuicTransportSend, WorkerReaper, connect_transport, create_frame_pipeline_manager_state,
        create_screen_capture_manager_state, create_screen_layout_manager_state, receive_request,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        connection_request::ConnectionRequest,
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        geometry::PixelSize,
        screen_configuration::{
            RemoteDisplayMode, ResolutionResult, ScreenResolutionOutcome, ScreenResolutionStatus,
            ScreenStreamRequest, ScreenStreamsConfigured, SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{Session, SessionId, SessionMessage, SessionRecv, SessionRole, SessionSend},
        session_control::{ControlMessage, OutgoingScreenList},
        transport::TransportRecv,
    },
};

mod view;

pub(crate) struct PendingHostSession {
    send: SessionSend<QuicTransportSend>,
    recv: SessionRecv<QuicTransportRecv>,
}

pub(crate) struct RootComponent {
    model: ApplicationModel,
    view: Child<RootView>,
    _connection_listener: compio::runtime::JoinHandle<()>,
    _logger_guard: LoggerGuard,
}

pub(crate) enum RootMessage {
    Noop,
    Close,
    ConnectDirect(String),
    DirectConnectionFinished(eros::Result<Option<QuicTransport>>),
    ConnectionRequest(PendingQuicConnectionRequest),
    ConnectionRequestSelectionChanged(Option<usize>),
    AcceptSelectedConnection(Option<usize>),
    RejectSelectedConnection(Option<usize>),
    ConnectionAccepted(eros::Result<QuicTransport>),
    InitialScreenListFinished {
        session: PendingHostSession,
        result: eros::Result<()>,
    },
    ConnectionRejected(eros::Result<()>),
    ConnectionRequestFailed(eros::ErrorUnion),
    ConnectionListenerFailed(eros::ErrorUnion),
    SessionMessageReceived(SessionId, SessionMessage),
    VideoFrameAvailable(SessionId, ScreenId),
    ScreenStreamConfigurationFinished {
        session_id: SessionId,
        streams: Vec<(ScreenId, FramePipelineParameters)>,
        result: eros::Result<()>,
    },
    ScreenStreamRequestFinished {
        session_id: SessionId,
        screen_id: ScreenId,
        frame_size: PixelSize,
        result: eros::Result<()>,
    },
    SessionClosed(SessionId),
    SessionFailed(SessionId, eros::ErrorUnion),
    ScreenStreamFinished(SessionId, ScreenId, u64, eros::Result<()>),
    RemoteScreenSelectionChanged(Option<usize>),
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
        send: SessionSend<QuicTransportSend>,
        recv: SessionRecv<R>,
        sender: &ComponentSender<Self>,
    ) where
        R: TransportRecv + 'static,
    {
        let received_video_frames = LatestVideoFrames::default();
        info!(
            event = "session_started",
            session_id = send.id().0,
            role = ?send.role(),
            "Session started"
        );
        self.model.sessions.push(RunningSession {
            send: Rc::new(send),
            screen_streams: Default::default(),
            received_video_frames: received_video_frames.clone(),
            _receiver: compio::runtime::spawn(receive_session(
                recv,
                received_video_frames,
                sender.clone(),
            )),
        });
    }

    fn replace_screen_stream(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
        sender: &ComponentSender<Self>,
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
        self.model.remove_session(id);
        self.refresh_remote_screen_list();
    }

    fn refresh_remote_screen_list(&mut self) {
        let mut entries = self
            .model
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
        self.model.remote_screen_entries.clear();
        self.model.remote_screen_entries.extend(
            entries
                .iter()
                .map(|(session_id, screen_id, _)| (*session_id, *screen_id)),
        );
        self.model.selected_remote_screen = None;
        self.view.post(RootViewMessage::SetRemoteScreens(
            entries.into_iter().map(|(_, _, entry)| entry).collect(),
        ));
    }

    fn set_connection_status(&mut self, status: impl Into<String>) {
        self.view
            .post(RootViewMessage::SetConnectionStatus(status.into()));
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

    fn refresh_connection_request_list(&mut self, selected: Option<usize>) {
        self.model.selected_connection_request = selected;
        self.view.post(RootViewMessage::SetConnectionRequests {
            entries: self
                .model
                .pending_connection_requests
                .iter()
                .map(|request| {
                    format!(
                        "{} - {}",
                        request.request().requester_name,
                        request.remote_address(),
                    )
                })
                .collect(),
            selected,
        });
    }

    fn take_selected_connection_request(
        &mut self,
        selected: Option<usize>,
    ) -> Option<PendingQuicConnectionRequest> {
        let selected = selected?;
        if selected >= self.model.pending_connection_requests.len() {
            return None;
        }

        let request = self.model.pending_connection_requests.remove(selected);
        let next = if self.model.pending_connection_requests.is_empty() {
            None
        } else {
            Some(selected.min(self.model.pending_connection_requests.len() - 1))
        };
        self.refresh_connection_request_list(next);

        Some(request)
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
        let view = Child::<RootView>::init(RootViewInit {
            local_port: local_address.port(),
        })
        .await?;

        Ok(Self {
            model: ApplicationModel::new(app, requester_name),
            view,
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
            self.view => {
                RootViewEvent::Close => RootMessage::Close,
                RootViewEvent::ConnectDirect(input) => RootMessage::ConnectDirect(input),
                RootViewEvent::ConnectionRequestSelected(selected) =>
                    RootMessage::ConnectionRequestSelectionChanged(selected),
                RootViewEvent::AcceptConnection(selected) =>
                    RootMessage::AcceptSelectedConnection(selected),
                RootViewEvent::RejectConnection(selected) =>
                    RootMessage::RejectSelectedConnection(selected),
                RootViewEvent::RemoteScreenSelected(selected) =>
                    RootMessage::RemoteScreenSelectionChanged(selected),
            },
        }
    }

    async fn update_children(&mut self) -> eros::Result<bool> {
        update_children!(self.view)
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
            RootMessage::ConnectDirect(input) => {
                let (remote_ip, remote_port) = match Self::parse_direct_target(&input) {
                    Ok(target) => target,
                    Err(error) => {
                        self.set_connection_status(format!("Invalid address: {error}"));
                        return Ok(true);
                    }
                };
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
                self.view.post(RootViewMessage::SetConnecting(true));
                self.set_connection_status("Connecting...");

                compio::runtime::spawn(async move {
                    let result =
                        connect_transport(&endpoint, remote_ip, remote_port, request).await;
                    connection_sender.post(RootMessage::DirectConnectionFinished(result));
                })
                .detach();

                Ok(true)
            }
            RootMessage::DirectConnectionFinished(result) => {
                self.view.post(RootViewMessage::SetConnecting(false));

                match result {
                    Ok(Some(transport)) => {
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Controller, transport);
                        let (send, recv) = session.split();

                        self.start_session(send, recv, sender);
                        self.set_connection_status("Connection accepted");
                    }
                    Ok(None) => self.set_connection_status("Connection rejected"),
                    Err(error) => self.set_connection_status(format!("Connection failed: {error}")),
                }

                Ok(true)
            }
            RootMessage::ConnectionRequest(request) => {
                let first_request = self.model.pending_connection_requests.is_empty();
                let selected = self.model.selected_connection_request;

                self.model.pending_connection_requests.push(request);
                self.refresh_connection_request_list(selected.or(first_request.then_some(0)));

                Ok(true)
            }
            RootMessage::ConnectionRequestSelectionChanged(selected) => {
                self.model.selected_connection_request = selected;
                Ok(false)
            }
            RootMessage::AcceptSelectedConnection(selected) => {
                let Some(request) = self.take_selected_connection_request(selected) else {
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
            RootMessage::RejectSelectedConnection(selected) => {
                let Some(request) = self.take_selected_connection_request(selected) else {
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
                        let id = self.model.next_session_id()?;
                        let session = Session::new(id, SessionRole::Host, transport);
                        let (send, recv) = session.split();
                        let screen_list = OutgoingScreenList::try_from(self.model.app.screens())?;
                        let session = PendingHostSession { send, recv };
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
                match result {
                    Ok(()) => self.start_session(session.send, session.recv, sender),
                    Err(error) => {
                        error!(error = ?error, "Failed to send the initial screen list")
                    }
                }

                Ok(false)
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
                Ok(false)
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

                Ok(false)
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
                    return Ok(false);
                }

                if !self
                    .model
                    .sessions
                    .iter()
                    .any(|session| session.send.id() == session_id)
                {
                    return Ok(false);
                }

                for (screen_id, parameters) in streams {
                    if let Err(error) =
                        self.replace_screen_stream(session_id, screen_id, parameters, sender)
                    {
                        error!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Failed to start screen stream"
                        );
                    }
                }

                Ok(false)
            }
            RootMessage::SessionClosed(id) => {
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
            RootMessage::RemoteScreenSelectionChanged(selected_index) => {
                let previous = self.model.selected_remote_screen;
                let selected = selected_index
                    .and_then(|index| self.model.remote_screen_entries.get(index))
                    .copied();

                self.model.selected_remote_screen = selected;

                if selected == previous {
                    return Ok(false);
                }

                if let Some((session_id, screen_id)) = selected {
                    let Some(frame_size) =
                        self.model
                            .remote_screens
                            .get(&session_id)
                            .and_then(|screens| {
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
                session_id,
                screen_id,
                frame_size,
                result,
            } => {
                if !self
                    .model
                    .sessions
                    .iter()
                    .any(|session| session.send.id() == session_id)
                {
                    return Ok(false);
                }

                if let Err(error) = result {
                    error!(
                        session_id = session_id.0,
                        screen_id = screen_id.0,
                        error = ?error,
                        "Failed to request screen stream"
                    );
                    self.remove_session(session_id);
                    self.set_connection_status(format!(
                        "Session {} screen request failed: {error}",
                        session_id.0
                    ));
                } else {
                    self.set_connection_status(format!(
                        "Requested session {} screen {} at {}x{}",
                        session_id.0, screen_id.0, frame_size.width, frame_size.height
                    ));
                }

                Ok(true)
            }
        }
    }

    fn render_children(&mut self) -> eros::Result<()> {
        self.view.render()
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

async fn receive_session<R>(
    mut session: SessionRecv<R>,
    received_video_frames: LatestVideoFrames,
    sender: ComponentSender<RootComponent>,
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
