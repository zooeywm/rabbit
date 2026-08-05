use std::{
    fmt,
    fs::{File, create_dir_all},
};

use directories::ProjectDirs;
use eros::Context;
use jiff::Zoned;
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{format::Writer, time::FormatTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::app::config::{LogLevel, LoggingConfig};

pub struct LoggingGuard {
    _console: WorkerGuard,
    _file: WorkerGuard,
}

struct JiffTimer;

impl FormatTime for JiffTimer {
    fn format_time(&self, writer: &mut Writer<'_>) -> fmt::Result {
        write!(writer, "{}", Zoned::now().strftime("%y%m%d-%H%M%S%.3f%:z"))
    }
}

fn make_filter(rust_log: Option<&str>, config_level: LogLevel) -> eros::Result<EnvFilter> {
    match rust_log {
        Some(rust_log) => Ok(EnvFilter::try_new(rust_log)
            .with_context(|| format!("Invalid RUST_LOG: {rust_log}"))?),

        None => Ok(EnvFilter::default().add_directive(LevelFilter::from(config_level).into())),
    }
}

fn log_file_name(now: &Zoned) -> String {
    let offset_seconds = now.offset().seconds();
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let offset_seconds = offset_seconds.unsigned_abs();
    let hours = offset_seconds / 3_600;
    let minutes = offset_seconds % 3_600 / 60;
    let seconds = offset_seconds % 60;

    let offset = if seconds != 0 {
        format!("{hours}{minutes:02}{seconds:02}")
    } else if minutes != 0 {
        format!("{hours}{minutes:02}")
    } else {
        hours.to_string()
    };

    format!(
        "rabbit-{}{sign}{offset}.log",
        now.strftime("%y%m%d-%H%M%S-%3f")
    )
}

pub fn init(project_dirs: &ProjectDirs, config: &LoggingConfig) -> eros::Result<LoggingGuard> {
    let log_dir = project_dirs
        .state_dir()
        .unwrap_or_else(|| project_dirs.data_local_dir());
    create_dir_all(log_dir)?;

    let log_path = log_dir.join(log_file_name(&Zoned::now()));
    let log_file = File::create(&log_path)?;

    let (console_writer, console_guard) = tracing_appender::non_blocking(std::io::stderr());

    let (file_writer, file_guard) = tracing_appender::non_blocking(log_file);

    let rust_log = std::env::var(EnvFilter::DEFAULT_ENV).ok();

    let console_filter = make_filter(rust_log.as_deref(), config.console_level)?;

    let file_filter = make_filter(rust_log.as_deref(), config.file_level)?;

    let console_layer = tracing_subscriber::fmt::layer()
        .with_timer(JiffTimer)
        .with_writer(console_writer)
        .with_filter(console_filter);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_timer(JiffTimer)
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(file_filter);

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .try_init()
        .with_context(|| "Failed to initialize logging")?;

    Ok(LoggingGuard {
        _console: console_guard,
        _file: file_guard,
    })
}

impl From<LogLevel> for LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => LevelFilter::TRACE,
            LogLevel::Debug => LevelFilter::DEBUG,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Error => LevelFilter::ERROR,
            LogLevel::Off => LevelFilter::OFF,
        }
    }
}
