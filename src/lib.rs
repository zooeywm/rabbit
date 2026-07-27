//! Rabbit application library.
//!
//! # Entry points
//!
//! - [`run`] — full GUI Host/Controller application
//! - [`run_headless`] — headless Host (auto-accept controllers, no GUI)
//! - [`run_record`] — local screen capture to an MP4 file (no network session)
//!
//! Domain types and protocol constants are available through [`kernel`].

pub mod kernel;

mod app;
mod infra;

#[cfg(test)]
mod architecture;

/// CLI types for the binary entrypoint.
pub mod cli {
    pub use crate::app::{Cli, Command, RecordOptions};
}

pub use app::{Cli, Command, RecordOptions};

/// Install process-wide SIGINT/SIGTERM handlers (idempotent).
///
/// First signal requests graceful shutdown; a second signal force-exits.
pub fn install_shutdown_handlers() {
    app::shutdown::install();
}

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

/// Records a local screen to a file using the host capture/encode path.
///
/// Output path comes from config `[recording].output_path` (default:
/// standard Videos directory / `rabbit` / timestamped `.mp4`). Screen and
/// duration come from [`RecordOptions`] (CLI).
pub fn run_record(options: RecordOptions) -> eros::Result<()> {
    app::run_record(options)
}
