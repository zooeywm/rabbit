use directories::ProjectDirs;
use eros::Context;
use rabbit::{config::Config, logging};

fn main() -> eros::Result<()> {
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
