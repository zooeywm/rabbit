//! Local screen recording — capture + encode + write MP4 (no network session).

use std::{io::BufRead as _, path::PathBuf, time::Duration};

use eros::Context as _;
use futures_util::StreamExt as _;
use tracing::info;

use crate::{
    app::{
        cli::RecordOptions,
        config::Config,
        init_logging,
        platform::{ApplicationStack, RunnableApp},
        shutdown,
    },
    infra::{
        ConnectionEndpoint, GbmFramePipelineFrame, WorkerReaper, record_frames_to_mp4,
        unsync_queue::UnsyncQueue,
    },
    kernel::{
        frame_pipeline::{FrameDelivery, FramePipelineManager, FramePipelineParameters},
        screen_manager::{Screen, ScreenLayoutManager},
    },
};

/// How long to wait for the first captured frame before failing (e.g. missing setcap).
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// Headless local recording for a concrete platform stack.
pub(crate) async fn run<Stack>(config: Config, options: RecordOptions) -> eros::Result<()>
where
    Stack: ApplicationStack,
    Stack::App: FramePipelineManager<Frame = GbmFramePipelineFrame>,
    <Stack::App as FramePipelineManager>::Subscription: Unpin,
{
    let _logger = init_logging(&config)?;
    let output_path = resolve_recording_output_path(&config)?;
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

    let mut frames = FramePipelineManager::subscribe(
        &mut app,
        &screen_id,
        parameters,
        frame_rate,
        FrameDelivery::recording(),
    )?;

    // Fail fast if KMS capture never produces a frame (missing CAP_SYS_ADMIN is common).
    let first_frame = match compio::time::timeout(FIRST_FRAME_TIMEOUT, frames.next()).await {
        Ok(Some(Ok(frame))) => frame,
        Ok(Some(Err(error))) => {
            return Err(error).context(capture_permission_hint(
                "Failed to capture the first recording frame",
            ));
        }
        Ok(None) => {
            eros::bail!(
                "{}",
                capture_permission_hint(
                    "Recording ended before the first frame arrived (capture closed)"
                )
            );
        }
        Err(_) => {
            eros::bail!(
                "{}",
                capture_permission_hint(&format!(
                    "Timed out after {}s waiting for the first captured frame",
                    FIRST_FRAME_TIMEOUT.as_secs()
                ))
            );
        }
    };

    let cancellation = UnsyncQueue::default();
    spawn_stop_triggers(cancellation.clone(), duration_secs, output_path.clone());

    record_frames_to_mp4(
        futures_util::stream::iter(std::iter::once(Ok(first_frame))).chain(frames),
        frame_rate,
        &output_path,
        cancellation,
    )
    .await
    .with_context(|| format!("Failed to record to {}", output_path.display()))?;

    info!(
        event = "local_record_complete",
        output = %output_path.display(),
        "Local screen recording complete"
    );
    Ok(())
}

fn capture_permission_hint(prefix: &str) -> String {
    format!(
        "{prefix}. KMS capture usually needs CAP_SYS_ADMIN on the rabbit binary \
(e.g. `make record` / `make setcap-release`, or \
`sudo setcap cap_sys_admin+ep target/release/rabbit`)."
    )
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

fn resolve_recording_output_path(config: &Config) -> eros::Result<PathBuf> {
    let configured = config.recording.output_path.trim();
    if configured.is_empty() {
        let base = default_videos_rabbit_dir()?;
        std::fs::create_dir_all(&base)
            .with_context(|| format!("Failed to create recording directory {}", base.display()))?;
        return Ok(base.join(default_recording_file_name()));
    }

    let path = PathBuf::from(expand_user_path(configured));
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp4" | "m4v" | "mov"
            )
        })
    {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create recording parent directory {}",
                    parent.display()
                )
            })?;
        }
        return Ok(path);
    }

    std::fs::create_dir_all(&path)
        .with_context(|| format!("Failed to create recording directory {}", path.display()))?;
    Ok(path.join(default_recording_file_name()))
}

fn default_videos_rabbit_dir() -> eros::Result<PathBuf> {
    if let Some(user_dirs) = directories::UserDirs::new()
        && let Some(videos) = user_dirs.video_dir()
    {
        return Ok(videos.join(crate::app::config::APP_NAME));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home)
            .join("Videos")
            .join(crate::app::config::APP_NAME));
    }
    Ok(PathBuf::from("Videos").join(crate::app::config::APP_NAME))
}

fn default_recording_file_name() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let format = time::macros::format_description!("[year][month][day]-[hour][minute][second]");
    let stamp = now.format(&format).unwrap_or_else(|_| "recording".into());
    format!("rabbit-{stamp}.mp4")
}

fn expand_user_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}{}{rest}", std::path::MAIN_SEPARATOR);
    }
    if path == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return home;
    }
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{default_videos_rabbit_dir, resolve_recording_output_path};
    use crate::app::config::Config;

    #[test]
    fn resolves_recording_output_file_or_directory() {
        let directory =
            std::env::temp_dir().join(format!("rabbit-record-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");

        let file = directory.join("clip.mp4");
        let mut config = Config::default();
        config.recording.output_path = file.to_string_lossy().into_owned();
        assert_eq!(
            resolve_recording_output_path(&config).expect("file path"),
            file
        );

        config.recording.output_path = directory.to_string_lossy().into_owned();
        let resolved = resolve_recording_output_path(&config).expect("directory path");
        assert_eq!(resolved.parent(), Some(directory.as_path()));
        assert!(
            resolved
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rabbit-") && name.ends_with(".mp4"))
        );

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn default_recording_directory_is_under_rabbit() {
        let directory = default_videos_rabbit_dir().expect("videos directory");
        assert_eq!(
            directory.file_name().and_then(|name| name.to_str()),
            Some("rabbit")
        );
    }
}
