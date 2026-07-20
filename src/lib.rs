//! Rabbit application library.

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit application.
pub fn run() -> eros::Result<()> {
    app::run_gui()
}
