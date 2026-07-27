//! Rabbit application library.
//!
//! # Entry points
//!
//! - [`run`] — full GUI Host/Controller application
//! - [`run_headless`] — headless Host (auto-accept controllers, no Slint)
//!
//! Domain types and protocol constants are available through [`kernel`].

pub mod kernel;

mod app;
mod infra;

/// Creates and runs the Rabbit GUI application.
pub fn run() -> eros::Result<()> {
    app::run_gui()
}

/// Runs a headless Host using the selected platform stack (no GUI).
///
/// Listens for controllers, auto-accepts connection requests, publishes the
/// local screen list, and serves screen streams via the same encode path as the
/// GUI host role. Application services under `app::services` are shared.
pub fn run_headless() -> eros::Result<()> {
    app::run_headless()
}
