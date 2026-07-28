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
        model::{ApplicationModel, RunningSession, SessionKey},
        platform::{ApplicationStack, RunnableApp},
        runtime::{
            host_control::{HostControlEffect, apply_host_session_message},
            host_stream_launch::launch_host_stream,
            host_stream_lifecycle::{ScreenStreamFinishEffect, apply_screen_stream_finished},
            session_lifecycle::{ReconnectEligibility, SessionTimeoutPolicy, evaluate_reconnect},
        },
        services::host_stream::HostStreamPlan,
        shutdown,
    },
    infra::{
        ConnectionEndpoint, PendingConnectionRequest, WorkerReaper, receive_request,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        connection_request::PeerCapabilities,
        frame_pipeline::FramePipelineManager,
        protocol::{PROTOCOL_NAME, protocol_version_string},
        screen_manager::{ScreenId, ScreenLayoutManager},
        session::{Session, SessionId, SessionMessage, SessionRecv, SessionRole},
        session_control::OutgoingScreenList,
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
    let local_capabilities = PeerCapabilities::local_host(ScreenLayoutManager::screens(&app).len());
    let mut model = ApplicationModel::new(app, requester_name, local_capabilities);
    let mut remote_input_injector = Stack::create_remote_input_injector();
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
    let timeout_policy = SessionTimeoutPolicy::default();

    let shutdown_rx = shutdown::subscribe();
    let shutdown_sender = sender.clone();
    compio::runtime::spawn(async move {
        let _ = shutdown_rx.recv_async().await;
        shutdown_sender.push(HeadlessMessage::Shutdown);
    })
    .detach();

    let listener = connection_endpoint.clone();
    let listener_sender = sender.clone();
    compio::runtime::spawn(async move {
        loop {
            if shutdown::is_requested() {
                return;
            }
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
        "Waiting for controller connection requests (auto-accept); Ctrl-C to stop"
    );

    loop {
        apply_headless_timeouts(&mut model, &timeout_policy);
        match messages.pop().await {
            HeadlessMessage::Shutdown => {
                info!(
                    event = "headless_shutdown_started",
                    "Headless host shutting down"
                );
                shutdown_headless(&mut model).await;
                info!(
                    event = "headless_shutdown_finished",
                    "Headless host stopped"
                );
                return Ok(());
            }
            HeadlessMessage::ListenerFailed(error) => {
                return Err(error).context("Headless connection listener stopped");
            }
            HeadlessMessage::Request(request) => {
                accept_request::<Stack>(&mut model, request, &sender).await?;
            }
            HeadlessMessage::SessionMessage(id, message) => {
                handle_session_message::<Stack>(
                    &mut model,
                    &mut remote_input_injector,
                    id,
                    message,
                    &sender,
                )?;
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
                    let closed_normally = session.send.is_closed_normally();
                    let effect = apply_screen_stream_finished(
                        &mut session.screen_streams,
                        screen_id,
                        stream_id,
                        result.is_err(),
                        closed_normally,
                    );
                    if effect != ScreenStreamFinishEffect::Stale {
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
    Shutdown,
}

async fn shutdown_headless<Stack>(model: &mut ApplicationModel<Stack>)
where
    Stack: ApplicationStack,
{
    let tasks = model.begin_screen_stream_shutdown();
    let sessions = model
        .sessions
        .iter()
        .map(|session| Rc::clone(&session.send))
        .collect::<Vec<_>>();

    for task in tasks {
        if let Err(error) = task.await {
            error!(error = ?error, "Screen stream task failed during headless shutdown");
        }
    }
    for session in sessions {
        session.close().await;
    }
    model.sessions.clear();
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
    let peer_capabilities = request.request().capabilities.clone();
    let remote = request.remote_address();
    info!(
        event = "headless_auto_accept",
        %remote,
        requester_name = %peer_name,
        max_screens = peer_capabilities.max_screens,
        "Auto-accepting controller connection"
    );

    let transport = request.accept(model.local_capabilities.clone()).await?;
    let id = model.next_session_id()?;
    let session = Session::new(id, SessionRole::Host, transport);
    let (send, recv) = session.split();
    let screen_list = OutgoingScreenList::try_from(ScreenLayoutManager::screens(&model.app))?;
    send.send_screen_list(screen_list)
        .await
        .context("Failed to send headless screen list")?;

    let key = SessionKey::new(remote, SessionRole::Host);
    let existing_phase = model.session_phase_for_key(&key);
    if evaluate_reconnect(existing_phase) == ReconnectEligibility::Denied {
        send.close().await;
        return Ok(());
    }
    if let Some(crate::app::model::SessionPhase::Draining) = existing_phase
        && let Some(old_id) = model
            .sessions
            .iter()
            .find(|session| session.key == key)
            .map(|session| session.send.id())
    {
        info!(
            event = "headless_session_reconnect_supersede",
            session_id = old_id.0,
            "Superseding draining headless session for reconnect"
        );
        model.remove_session(old_id);
    }

    let mut running = RunningSession::new(
        key,
        Some(peer_name),
        peer_capabilities,
        Rc::new(send),
        compio::runtime::spawn(receive_session(recv, sender.clone())),
    );
    running
        .activate()
        .context("Failed to activate headless host session")?;
    model.sessions.push(running);
    Ok(())
}

fn apply_headless_timeouts<Stack>(
    model: &mut ApplicationModel<Stack>,
    policy: &SessionTimeoutPolicy,
) where
    Stack: ApplicationStack,
{
    let timed_out: Vec<_> = model
        .sessions
        .iter()
        .filter(|session| session.peek_timeout(policy).is_some())
        .map(|session| session.send.id())
        .collect();
    for id in timed_out {
        info!(
            event = "headless_session_timeout",
            session_id = id.0,
            "Headless session hit timeout policy; removing"
        );
        model.remove_session(id);
    }
}

fn handle_session_message<Stack>(
    model: &mut ApplicationModel<Stack>,
    remote_input_injector: &mut Stack::RemoteInputInjector,
    id: SessionId,
    message: SessionMessage,
    sender: &UnsyncQueue<HeadlessMessage>,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
    <Stack::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
{
    let effect = apply_host_session_message(model, remote_input_injector, id, message);
    match effect {
        HostControlEffect::ConfigureStreams(configuration) => {
            let finished = sender.clone();
            compio::runtime::spawn(async move {
                let completion = configuration.send().await;
                finished.push(HeadlessMessage::ConfigurationFinished {
                    session_id: completion.session_id,
                    plans: completion.plans,
                    result: completion.result,
                });
            })
            .detach();
        }
        HostControlEffect::StreamRequestRejected(_)
        | HostControlEffect::StreamStopped(_)
        | HostControlEffect::NoShellAction => {}
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
    let screen_id = plan.screen_id;
    let finished = sender.clone();
    launch_host_stream(
        model,
        session_id,
        plan,
        move |session_id, screen_id, stream_id, result| {
            finished.push(HeadlessMessage::ScreenStreamFinished(
                session_id, screen_id, stream_id, result,
            ));
        },
    )?;
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
