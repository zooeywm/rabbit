mod config;
pub(crate) mod container;
mod logging;
mod runtime;

use config::Config;
use directories::ProjectDirs;
use eros::Context;
use runtime::{AppActor, AppRuntime};

pub(crate) fn run<App>(
    create_app: impl FnOnce() -> eros::Result<App> + Send + 'static,
) -> eros::Result<()>
where
    App: AppActor + 'static,
{
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;

    let app_handle = AppRuntime::start(create_app)?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    app_handle.shutdown()
}
