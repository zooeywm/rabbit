mod config;
pub(crate) mod container;
mod logging;
mod root_runtime;

use config::Config;
use directories::ProjectDirs;
use eros::Context;
use root_runtime::RootHandle;

pub(crate) fn run<Root>(
    create_root: impl FnOnce() -> eros::Result<Root> + Send + 'static,
) -> eros::Result<()>
where
    Root: 'static,
{
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;

    let root_handle = RootHandle::start(create_root)?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    root_handle.shutdown()
}
