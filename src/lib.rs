//! Rabbit application library.

use eros::Context;

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit application.
pub async fn run() -> eros::Result<()> {
    let config = app::config::Config::new()?;
    app::init_logging(&config)?;
    let screen_layout_manager_state = infra::create_screen_layout_manager_state()
        .context("Failed to create the screen layout manager state")?;
    let rayon_thread_pool_state = infra::RayonThreadPoolState::new()
        .context("Failed to create the Rayon thread pool state")?;
    let mut app = app::App::<infra::NiriScreenLayoutManagerState>::new(
        config,
        screen_layout_manager_state,
        rayon_thread_pool_state,
    );
    app.run().await
}
