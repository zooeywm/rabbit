//! Rabbit application library.

use eros::Context;

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit application.
pub fn run() -> eros::Result<()> {
    let screen_layout_manager = infra::create_screen_layout_manager_state()
        .context("Failed to create the screen layout manager state")?;
    let mut app = app::App::<infra::NiriScreenLayoutManagerState>::new(screen_layout_manager)?;
    app.run()
}
