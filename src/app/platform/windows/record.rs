use crate::app::{RecordOptions, config::Config, init_logging};

pub(super) fn run(config: Config, _options: RecordOptions) -> eros::Result<()> {
    let _ = init_logging(&config)?;
    eros::bail!("Local screen recording (`rabbit record`) is currently supported on Linux only")
}
