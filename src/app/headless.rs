//! Headless host runtime — no Slint, reuses stack assembly and services.
//!
//! Starts the selected platform stack, logs local screens, listens for
//! connection requests, auto-accepts them as Host, and serves screen streams
//! using the same encode path as the GUI host role.

use std::rc::Rc;

use eros::Context as _;
use tracing::{error, info, warn};

use crate::{
    app::{
        config::Config,
        init_logging,
        model::{ApplicationModel, RunningScreenStream, RunningSession, SessionKey},
        platform::{ApplicationStack, RunnableApp},
        screen_stream::run_host_screen_stream,
        services::host_stream::{HostStreamPlan, plan_preserved_streams},
    },
    infra::{
        ConnectionEndpoint, PendingConnectionRequest, WorkerReaper, receive_request,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        frame_pipeline::FramePipelineManager,
        protocol::{PROTOCOL_NAME, protocol_version_string},
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{Session, SessionId, SessionMessage, SessionRecv, SessionRole},
        session_control::{ControlMessage, OutgoingScreenList},
        transport::TransportRecv,
        video_encoder::VideoEncoder,
    },
};

/// Headless Host message loop for a concrete platform stack.
pub(crate) async fn run<Stack>(config: Config) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
    <Stack::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
{
    let _logger = init_logging(&config)?;
    let (worker_reaper, worker_reaper_handle) =
        WorkerReaper::new().context("Failed to start the background worker reaper")?;
    let connection_endpoint = ConnectionEndpoint::new(config.network.transport)
        .await
        .context("Failed to create the configured connection endpoint")?;
    let local_address = connection_endpoint.local_address()?;
    let requester_name = format!("{}-headless ({})", config.app_name, local_address.port());

    info!(
        event = "headless_host_started",
        stack = Stack::name(),
        protocol = PROTOCOL_NAME,
        protocol_version = protocol_version_string(),
        transport = ?config.network.transport,
        %local_address,
        "Headless host started"
    );

    let mut app = Stack::create_app(
        config,
        connection_endpoint.clone(),
        worker_reaper,
        worker_reaper_handle,
    )?;
    app.run_app().await?;

    let mut model = ApplicationModel::new(app, requester_name);
    let screens = ScreenLayoutManager::screens(&model.app).to_vec();
    info!(
        event = "headless_screens_detected",
        count = screens.len(),
        "Detected local screens"
    );
    for screen in &screens {
        info!(
            event = "headless_screen",
            id = screen.id.get(),
            name = %screen.name,
            width = screen.resolution.width,
            height = screen.resolution.height,
            "Local screen"
        );
    }

    let messages = UnsyncQueue::<HeadlessMessage>::default();
    let sender = messages.clone();

    let listener = connection_endpoint.clone();
    let listener_sender = sender.clone();
    compio::runtime::spawn(async move {
        loop {
            let connection = match listener.accept_connection().await {
                Ok(Some(connection)) => connection,
                Ok(None) => return,
                Err(error) => {
                    listener_sender.push(HeadlessMessage::ListenerFailed(error));
                    return;
                }
            };
            match receive_request(connection).await {
                Ok(Some(request)) => listener_sender.push(HeadlessMessage::Request(request)),
                Ok(None) => {}
                Err(error) => {
                    warn!(error = ?error, "Failed to receive headless connection request");
                }
            }
        }
    })
    .detach();

    info!(
        event = "headless_listener_ready",
        %local_address,
        "Waiting for controller connection requests (auto-accept)"
    );

    loop {
        match messages.pop().await {
            HeadlessMessage::ListenerFailed(error) => {
                return Err(error).context("Headless connection listener stopped");
            }
            HeadlessMessage::Request(request) => {
                accept_request::<Stack>(&mut model, request, &sender).await?;
            }
            HeadlessMessage::SessionMessage(id, message) => {
                handle_session_message::<Stack>(&mut model, id, message, &sender).await?;
            }
            HeadlessMessage::SessionClosed(id) => {
                info!(session_id = id.0, "Headless session closed");
                model.remove_session(id);
            }
            HeadlessMessage::SessionFailed(id, error) => {
                error!(session_id = id.0, error = ?error, "Headless session failed");
                model.remove_session(id);
            }
            HeadlessMessage::ScreenStreamFinished(id, screen_id, stream_id, result) => {
                if let Some(session) = model
                    .sessions
                    .iter_mut()
                    .find(|session| session.send.id() == id)
                {
                    let current = session
                        .screen_streams
                        .get(&screen_id)
                        .is_some_and(|stream| stream.id == stream_id);
                    if current {
                        session.screen_streams.remove(&screen_id);
                        match result {
                            Ok(()) => info!(
                                session_id = id.0,
                                screen_id = screen_id.0,
                                "Headless screen stream finished"
                            ),
                            Err(error) => error!(
                                session_id = id.0,
                                screen_id = screen_id.0,
                                error = ?error,
                                "Headless screen stream failed"
                            ),
                        }
                    }
                }
            }
            HeadlessMessage::ConfigurationFinished {
                session_id,
                plans,
                result,
            } => {
                if let Err(error) = result {
                    error!(session_id = session_id.0, error = ?error, "Failed to send stream config");
                    model.remove_session(session_id);
                    continue;
                }
                for plan in plans {
                    let screen_id = plan.screen_id;
                    if let Err(error) =
                        start_host_stream::<Stack>(&mut model, session_id, plan, &sender)
                    {
                        error!(
                            session_id = session_id.0,
                            screen_id = screen_id.0,
                            error = ?error,
                            "Failed to start headless host stream"
                        );
                    }
                }
            }
        }
    }
}

enum HeadlessMessage {
    Request(PendingConnectionRequest),
    SessionMessage(SessionId, SessionMessage),
    SessionClosed(SessionId),
    SessionFailed(SessionId, eros::ErrorUnion),
    ScreenStreamFinished(SessionId, ScreenId, u64, eros::Result<()>),
    ConfigurationFinished {
        session_id: SessionId,
        plans: Vec<HostStreamPlan>,
        result: eros::Result<()>,
    },
    ListenerFailed(eros::ErrorUnion),
}

async fn accept_request<Stack>(
    model: &mut ApplicationModel<Stack>,
    request: PendingConnectionRequest,
    sender: &UnsyncQueue<HeadlessMessage>,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
{
    let peer_name = request.request().requester_name.clone();
    let remote = request.remote_address();
    info!(
        event = "headless_auto_accept",
        %remote,
        requester_name = %peer_name,
        max_screens = request.request().capabilities.max_screens,
        "Auto-accepting controller connection"
    );

    let transport = request.accept().await?;
    let id = model.next_session_id()?;
    let session = Session::new(id, SessionRole::Host, transport);
    let (send, recv) = session.split();
    let screen_list = OutgoingScreenList::try_from(ScreenLayoutManager::screens(&model.app))?;
    send.send_screen_list(screen_list)
        .await
        .context("Failed to send headless screen list")?;

    let key = SessionKey::new(remote, SessionRole::Host);
    if model.has_session(&key) {
        send.close().await;
        return Ok(());
    }

    let mut running = RunningSession::new(
        key,
        Some(peer_name),
        Rc::new(send),
        compio::runtime::spawn(receive_session(recv, sender.clone())),
    );
    running.activate();
    model.sessions.push(running);
    Ok(())
}

async fn handle_session_message<Stack>(
    model: &mut ApplicationModel<Stack>,
    id: SessionId,
    message: SessionMessage,
    sender: &UnsyncQueue<HeadlessMessage>,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
    <Stack::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
{
    match message {
        SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
            let (configured, plans) =
                plan_preserved_streams(request, |screen_id| model.app.screen(screen_id).cloned());
            let Some(session) = model
                .sessions
                .iter()
                .find(|session| session.send.id() == id && session.admits_new_streams())
            else {
                warn!(session_id = id.0, "No active session for stream request");
                return Ok(());
            };
            let session_send = Rc::clone(&session.send);
            let finished = sender.clone();
            compio::runtime::spawn(async move {
                let result = session_send
                    .send_screen_streams_configured(configured)
                    .await;
                finished.push(HeadlessMessage::ConfigurationFinished {
                    session_id: id,
                    plans,
                    result,
                });
            })
            .detach();
        }
        SessionMessage::Control(ControlMessage::RequestKeyFrame(request)) => {
            if let Some(session) = model
                .sessions
                .iter_mut()
                .find(|session| session.send.id() == id)
                && let Some(stream) = session.screen_streams.get(&request.screen_id)
            {
                stream.request_key_frame();
            }
        }
        SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
            if let Some(session) = model
                .sessions
                .iter_mut()
                .find(|session| session.send.id() == id)
            {
                session.screen_streams.remove(&stop.screen_id);
            }
        }
        SessionMessage::Control(other) => {
            warn!(
                session_id = id.0,
                message = ?std::mem::discriminant(&other),
                "Headless host ignored control message"
            );
        }
        SessionMessage::Video(_) | SessionMessage::KeyFrameRequired(_) => {
            warn!(
                session_id = id.0,
                "Headless host ignored controller media message"
            );
        }
    }
    Ok(())
}

fn start_host_stream<Stack>(
    model: &mut ApplicationModel<Stack>,
    session_id: SessionId,
    plan: HostStreamPlan,
    sender: &UnsyncQueue<HeadlessMessage>,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
    <Stack::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
{
    let frames = FramePipelineManager::subscribe(
        &mut model.app,
        &plan.screen_id,
        plan.parameters,
        plan.frame_rate,
    )?;
    let stream_id = model.next_screen_stream_id()?;
    let Some(session) = model
        .sessions
        .iter_mut()
        .find(|session| session.send.id() == session_id)
    else {
        eros::bail!("Session {} missing while starting stream", session_id.0);
    };
    if !session.admits_new_streams() {
        eros::bail!("Session {} does not admit new streams", session_id.0);
    }

    let session_send = Rc::clone(&session.send);
    let cancellation = UnsyncQueue::default();
    let task_cancellation = cancellation.clone();
    let encoder_commands = UnsyncQueue::default();
    let task_encoder_commands = encoder_commands.clone();
    let finished = sender.clone();
    let screen_id = plan.screen_id;
    let frame_rate = plan.frame_rate;
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
        finished.push(HeadlessMessage::ScreenStreamFinished(
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

    info!(
        event = "headless_stream_started",
        session_id = session_id.0,
        screen_id = screen_id.0,
        "Headless host screen stream started"
    );
    Ok(())
}

async fn receive_session<R>(mut session: SessionRecv<R>, sender: UnsyncQueue<HeadlessMessage>)
where
    R: TransportRecv,
{
    let id = session.id();
    loop {
        match session.recv().await {
            Ok(Some(message)) => sender.push(HeadlessMessage::SessionMessage(id, message)),
            Ok(None) => {
                sender.push(HeadlessMessage::SessionClosed(id));
                return;
            }
            Err(error) => {
                sender.push(HeadlessMessage::SessionFailed(id, error));
                return;
            }
        }
    }
}
