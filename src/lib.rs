//! Rabbit application library.

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit application.
pub fn run() -> eros::Result<()> {
    use winio::prelude::ComponentExt;

    let gui = winio::ui::App::builder()
        .name(app::config::APP_NAME)
        .build()?;
    gui.block_on(app::RootComponent::run_until_event(()))?;
    Ok(())
}
