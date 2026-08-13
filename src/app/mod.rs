mod config;
pub(crate) mod container;
mod logging;
mod metrics;
mod runtime;

use config::Config;
use directories::ProjectDirs;
use eros::Context;
use runtime::AppRuntime;

pub(crate) use runtime::{AppHandle, AppMessage};

pub(crate) struct RunningApp {
    app_runtime: AppRuntime,
    _logging_guard: logging::LoggingGuard,
    _metrics_guard: metrics::MetricsGuard,
}

pub(crate) fn run() -> eros::Result<RunningApp> {
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let logging_guard = logging::init(&project_dirs, &config.logging)?;
    let metrics_guard = metrics::init();
    let app_runtime = AppRuntime::start()?;

    tracing::info!("rabbit started");

    Ok(RunningApp {
        app_runtime,
        _logging_guard: logging_guard,
        _metrics_guard: metrics_guard,
    })
}

impl RunningApp {
    pub(crate) fn handle(&self) -> AppHandle {
        self.app_runtime.handle()
    }

    pub(crate) fn shutdown(self) -> eros::Result<()> {
        self.app_runtime.shutdown()
    }
}
