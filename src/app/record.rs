//! Local screen recording — capture + encode + write MP4 (no network session).

use std::{io::BufRead as _, path::PathBuf, time::Duration};

use eros::Context as _;
use tracing::info;

use crate::{
    app::{
        cli::RecordOptions,
        config::Config,
        init_logging,
        platform::{ApplicationStack, RunnableApp},
        shutdown,
    },
    infra::{ConnectionEndpoint, WorkerReaper, unsync_queue::UnsyncQueue},
    kernel::{
        frame_pipeline::{FrameDelivery, FramePipelineManager, FramePipelineParameters},
        screen_manager::{Screen, ScreenLayoutManager},
    },
};

/// Headless local recording for a concrete platform stack.
#[cfg(target_os = "linux")]
pub(crate) async fn run<Stack>(config: Config, options: RecordOptions) -> eros::Result<()>
where
    Stack: ApplicationStack,
    Stack::App: FramePipelineManager<Frame = crate::infra::GbmFramePipelineFrame>,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
{
    use crate::infra::record_frames_to_mp4;

    let _logger = init_logging(&config)?;
    let output_path = config.resolve_recording_output_path()?;
    let duration_secs = options.duration_secs.unwrap_or(0);
    let screen_filter = options.screen.unwrap_or_default();

    let (worker_reaper, worker_reaper_handle) =
        WorkerReaper::new().context("Failed to start the background worker reaper")?;
    // Stack assembly still requires a connection endpoint; recording does not listen.
    let connection_endpoint = ConnectionEndpoint::new(config.network.transport)
        .await
        .context("Failed to create the configured connection endpoint")?;

    info!(
        event = "local_record_starting",
        stack = Stack::name(),
        output = %output_path.display(),
        screen = %screen_filter,
        duration_secs,
        "Starting local screen recording"
    );

    let mut app = Stack::create_app(
        config,
        connection_endpoint,
        worker_reaper,
        worker_reaper_handle,
    )?;
    app.run_app().await?;

    let screen = select_screen(&app, &screen_filter)?;
    let frame_rate = screen.frame_rate;
    let parameters = FramePipelineParameters {
        frame_size: screen.resolution,
    };
    let screen_id = screen.id;
    info!(
        event = "local_record_screen_selected",
        id = screen_id.get(),
        name = %screen.name,
        width = screen.resolution.width,
        height = screen.resolution.height,
        fps_num = frame_rate.numerator(),
        fps_den = frame_rate.denominator(),
        "Selected screen for local recording"
    );

    let frames = FramePipelineManager::subscribe(
        &mut app,
        &screen_id,
        parameters,
        frame_rate,
        FrameDelivery::recording(),
    )?;
    let cancellation = UnsyncQueue::default();
    spawn_stop_triggers(cancellation.clone(), duration_secs, output_path.clone());

    record_frames_to_mp4(frames, frame_rate, &output_path, cancellation)
        .await
        .with_context(|| format!("Failed to record to {}", output_path.display()))?;

    info!(
        event = "local_record_complete",
        output = %output_path.display(),
        "Local screen recording complete"
    );
    Ok(())
}

/// Headless local recording for a concrete platform stack.
#[cfg(not(target_os = "linux"))]
pub(crate) async fn run<Stack>(config: Config, _options: RecordOptions) -> eros::Result<()>
where
    Stack: ApplicationStack,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
{
    let _ = init_logging(&config)?;
    eros::bail!("Local screen recording (`rabbit record`) is currently supported on Linux only")
}

fn select_screen<App>(app: &App, configured_name: &str) -> eros::Result<Screen>
where
    App: ScreenLayoutManager,
{
    let screens = app.screens();
    if screens.is_empty() {
        eros::bail!("No local screens detected for recording");
    }
    let name = configured_name.trim();
    if name.is_empty() {
        return Ok(app.primary_screen()?.clone());
    }
    let Some(screen) = screens.iter().find(|screen| screen.name == name).cloned() else {
        let available = screens
            .iter()
            .map(|screen| screen.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eros::bail!("No screen named `{}` (available: {})", name, available);
    };
    Ok(screen)
}

fn spawn_stop_triggers(cancellation: UnsyncQueue<()>, duration_secs: u64, output_path: PathBuf) {
    // Fan-in: process shutdown (Ctrl-C), optional duration, optional Enter.
    let (stop_tx, stop_rx) = flume::bounded::<()>(1);

    {
        let tx = stop_tx.clone();
        let shutdown_rx = shutdown::subscribe();
        std::thread::Builder::new()
            .name("rabbit-record-shutdown-bridge".into())
            .spawn(move || {
                let _ = shutdown_rx.recv();
                eprintln!("Stop signal received — finalizing recording…");
                let _ = tx.send(());
            })
            .expect("Failed to spawn recording shutdown bridge");
    }

    if duration_secs > 0 {
        let tx = stop_tx.clone();
        let path = output_path.clone();
        compio::runtime::spawn(async move {
            compio::time::sleep(Duration::from_secs(duration_secs)).await;
            info!(
                event = "local_record_duration_elapsed",
                duration_secs,
                output = %path.display(),
                "Recording duration elapsed"
            );
            shutdown::request();
            let _ = tx.send(());
        })
        .detach();
        eprintln!(
            "Recording to {} — stops after {duration_secs}s, or press Ctrl-C to stop early.",
            output_path.display()
        );
    } else {
        let tx = stop_tx;
        std::thread::Builder::new()
            .name("rabbit-record-stdin-stop".into())
            .spawn(move || {
                eprintln!(
                    "Recording to {} — press Enter or Ctrl-C to stop (do not force-kill).",
                    output_path.display()
                );
                let stdin = std::io::stdin();
                let mut line = String::new();
                let _ = stdin.lock().read_line(&mut line);
                shutdown::request();
                let _ = tx.send(());
            })
            .expect("Failed to spawn recording stdin-stop thread");
    }

    compio::runtime::spawn(async move {
        let _ = stop_rx.recv_async().await;
        cancellation.push(());
    })
    .detach();
}
