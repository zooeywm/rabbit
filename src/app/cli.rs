//! Static CLI surface (clap derive).

use clap::{Parser, Subcommand};

/// Rabbit command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "rabbit",
    about = "Peer-to-peer remote desktop",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Headless Host: auto-accept controllers (no GUI)
    #[command(visible_alias = "H")]
    Headless,

    /// Record a local screen to an MP4 file (path from config `[recording].output_path`)
    #[command(visible_alias = "R")]
    Record {
        /// Screen name (e.g. HDMI-A-1). Default: primary screen.
        #[arg(short, long)]
        screen: Option<String>,

        /// Stop after N seconds. Omit to stop when Enter is pressed.
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
