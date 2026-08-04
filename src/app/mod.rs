mod config;
mod logging;

mod container;

pub(crate) mod outbound_port;

mod pipeline;

pub use container::Container;

use directories::ProjectDirs;
use eros::Context;

use config::Config;

pub(crate) fn run() -> eros::Result<()> {
    let project_dirs =
        ProjectDirs::from("", "", "rabbit").context("Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    Ok(())
}
