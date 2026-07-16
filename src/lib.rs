//! Rabbit application library.

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit application.
pub fn run() -> eros::Result<()> {
    let gui = winio::ui::App::new(app::config::APP_NAME)?;
    gui.run_until_event::<app::RootComponent>(())?;
    Ok(())
}
