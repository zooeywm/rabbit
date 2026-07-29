//! Shared host stream task launch for GUI and headless shells.

use std::rc::Rc;

use crate::{
    app::{
        model::{ApplicationModel, RunningScreenStream},
        platform::ApplicationStack,
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

/// Subscribes the frame pipeline, spawns the encode/send task, and registers it
/// on the session. `report_finished` is invoked on the app runtime thread after
/// the task completes.
pub fn launch_host_stream<Stack, ReportFinished>(
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
    if !session.admits_new_streams() {
        eros::bail!(
            "Session {} is {:?} and cannot start screen {} stream",
            session_id.0,
            session.phase,
            screen_id.0
        );
    }

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
