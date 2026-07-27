//! Static CLI surface (palc derive — clap-compatible, static-first).

use palc::{Parser, Subcommand};

/// Peer-to-peer remote desktop.
#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Headless Host: auto-accept controllers (no GUI)
    Headless,

    /// Record a local screen to an MP4 file
    ///
    /// Output path comes from config `[recording].output_path`
    /// (default: standard Videos directory under `rabbit/`).
    Record {
        /// Screen name (e.g. HDMI-A-1). Default: primary screen.
        #[arg(short, long)]
        screen: Option<String>,

        /// Stop after N seconds. Omit to stop on Enter or Ctrl-C (graceful finalize).
        #[arg(short, long)]
        duration: Option<u64>,
    },
}

/// Options for `rabbit record` (everything except the config path).
#[derive(Debug, Clone, Default)]
pub struct RecordOptions {
    pub screen: Option<String>,
    pub duration_secs: Option<u64>,
}

impl RecordOptions {
    pub fn from_cli(screen: Option<String>, duration: Option<u64>) -> Self {
        Self {
            screen,
            duration_secs: duration,
        }
    }
}
