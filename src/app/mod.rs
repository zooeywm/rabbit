mod config;
pub(crate) mod container;
mod logging;

use config::Config;
use directories::ProjectDirs;
use eros::Context;

pub(crate) fn run<Root>(create_root: impl FnOnce() -> eros::Result<Root>) -> eros::Result<()> {
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;

    let _root_container = create_root()?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    Ok(())
}
