//! Shared host stream task launch for GUI and headless shells.

use std::rc::Rc;

use tracing::error;

use crate::{
    app::{
        model::{ApplicationModel, RunningScreenStream},
        platform::ApplicationStack,
        runtime::host_stream_lifecycle::begin_host_screen_stream_replacement,
        screen_stream::run_host_screen_stream,
        services::host_stream::HostStreamPlan,
    },
    infra::{SessionTransportSend, unsync_queue::UnsyncQueue},
    kernel::{
        frame_pipeline::{FrameDelivery, FramePipelineManager},
        session::{SessionId, SessionSend},
        video_encoder::VideoEncoder,
    },
};

pub fn notify_failed_host_stream(
    session: Rc<SessionSend<SessionTransportSend>>,
    session_id: SessionId,
    screen_id: crate::kernel::screen_manager::ScreenId,
) {
    compio::runtime::spawn(async move {
        if let Err(notify_error) = session.stop_screen_stream(screen_id).await {
            error!(
                event = "failed_stream_stop_notification_failed",
                session_id = session_id.0,
                screen_id = screen_id.0,
                error = ?notify_error,
                "Failed to notify the controller after the Host screen stream failed"
            );
        }
    })
    .detach();
}

/// Stops and joins the previous stream for the same screen, then subscribes the
/// replacement frame pipeline and registers its encode/send task.
/// `report_finished` is invoked on the app runtime thread after the task
/// completes.
pub async fn launch_host_stream<Stack, ReportFinished>(
    model: &mut ApplicationModel<Stack>,
    session_id: SessionId,
    plan: HostStreamPlan,
    report_finished: ReportFinished,
) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
    <Stack::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
    ReportFinished:
        FnOnce(SessionId, crate::kernel::screen_manager::ScreenId, u64, eros::Result<()>) + 'static,
{
    let screen_id = plan.screen_id;
    let encoding = plan.encoding;
    let frame_rate = encoding.frame_rate;
    let previous_task = {
        let Some(session) = model
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
        begin_host_screen_stream_replacement(&mut session.screen_streams, screen_id)
    };
    if let Some(task) = previous_task
        && let Err(error) = task.await
    {
        eros::bail!(
            "Failed to join the previous Session {} screen {} stream: {error:?}",
            session_id.0,
            screen_id.0
        );
    }

    let frames = FramePipelineManager::subscribe(
        &mut model.app,
        &screen_id,
        plan.parameters,
        frame_rate,
        FrameDelivery::Latest,
    )?;
    let stream_id = model.next_screen_stream_id()?;
    let Some(session) = model
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
    let session_send: Rc<SessionSend<SessionTransportSend>> = Rc::clone(&session.send);
    let cancellation = UnsyncQueue::default();
    let task_cancellation = cancellation.clone();
    let encoder_commands = UnsyncQueue::default();
    let task_encoder_commands = encoder_commands.clone();
    let task = compio::runtime::spawn(async move {
        let result = run_host_screen_stream::<_, _, Stack::ScreenStreamEncoder>(
            frames,
            screen_id,
            session_send,
            task_cancellation,
            task_encoder_commands,
            encoding,
        )
        .await;
        report_finished(session_id, screen_id, stream_id, result);
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
