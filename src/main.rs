use palc::Parser as _;

use rabbit::cli::{Cli, Command, RecordOptions};

fn main() -> eros::Result<()> {
    // Shared SIGINT/SIGTERM → graceful shutdown for GUI / headless / record.
    rabbit::install_shutdown_handlers();

    let cli = Cli::parse();
    match cli.command {
        None => rabbit::run(),
        Some(Command::Headless) => rabbit::run_headless(),
        Some(Command::Record { screen, duration }) => {
            rabbit::run_record(RecordOptions::from_cli(screen, duration))
        }
    }
}
