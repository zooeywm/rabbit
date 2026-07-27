use clap::Parser as _;

use rabbit::cli::{Cli, Command, RecordOptions};

fn main() -> eros::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => rabbit::run(),
        Some(Command::Headless) => rabbit::run_headless(),
        Some(Command::Record { screen, duration }) => {
            rabbit::run_record(RecordOptions::from_cli(screen, duration))
        }
    }
}
