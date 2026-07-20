mod file_writer;

use std::{
    fs::{OpenOptions, create_dir_all},
    io,
};

use eros::Context;
use time::{OffsetDateTime, macros::format_description};
use tracing_subscriber::{
    Layer,
    filter::LevelFilter,
    fmt::{self, format::Writer, time::FormatTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::app::{
    config::{Config, LogLevel},
    logging::file_writer::start_logger,
};

pub(crate) use crate::app::logging::file_writer::LoggerGuard;

struct LocalTimeWithOffset;

impl FormatTime for LocalTimeWithOffset {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        let timestamp = now
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3][offset_hour sign:mandatory]:[offset_minute]"
            ))
            .map_err(|_| std::fmt::Error)?;

        write!(w, "{timestamp}")
    }
}

pub(crate) fn init_logging(config: &Config) -> eros::Result<LoggerGuard> {
    let project_dirs = config
        .project_dirs
        .as_ref()
        .with_context(|| "Application project directories are not initialized")?;
    let log_base_dir = project_dirs
        .state_dir()
        .unwrap_or_else(|| project_dirs.data_local_dir());
    create_dir_all(log_base_dir).context("Failed create log dir")?;

    let now = OffsetDateTime::now_utc();

    let timestamp = now
        .format(format_description!(
            "[year][month][day]-[hour][minute][second]Z"
        ))
        .context("Failed formatting log file name")?;
    let app_name = config.app_name;

    let log_file_name = format!("{app_name}-{timestamp}.log");

    let log_file_path = log_base_dir.join(log_file_name);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .context("Failed open log file")?;
    let (file_writer, logger_guard) =
        start_logger(log_file).context("Failed to start Logger thread")?;

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_timer(LocalTimeWithOffset)
                .with_writer(io::stderr)
                .with_filter(level_filter(config.logging.console_level)),
        )
        .with(
            fmt::layer()
                .with_timer(LocalTimeWithOffset)
                .with_ansi(false)
                .with_writer(move || file_writer.make_writer())
                .with_filter(level_filter(config.logging.file_level)),
        )
        .try_init()
        .context("Failed init logging")?;
    Ok(logger_guard)
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Trace => LevelFilter::TRACE,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}
