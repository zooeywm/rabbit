use std::{
    fs::File,
    io::{self, Write},
    mem,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, unbounded};
use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LogMessage {
    Line {
        bytes: Vec<u8>,
        destinations: LogDestinations,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) struct ChannelLogMakeWriter {
    sender: Sender<LogMessage>,
    file_level: Level,
    stdout_level: Level,
}

#[derive(Debug)]
pub(crate) struct ChannelLogWriter {
    sender: Sender<LogMessage>,
    destinations: LogDestinations,
    line: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogDestinations {
    file: bool,
    stdout: bool,
}

#[derive(Debug)]
pub(crate) struct LoggerGuard {
    sender: Sender<LogMessage>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

pub(crate) fn start_logger(
    file: File,
    file_level: Level,
    stdout_level: Level,
) -> io::Result<(ChannelLogMakeWriter, LoggerGuard)> {
    let (sender, receiver) = unbounded();
    let thread = thread::Builder::new()
        .name("rabbit-logger".to_owned())
        .spawn(move || run_logger(file, io::stdout(), receiver))?;

    Ok((
        ChannelLogMakeWriter {
            sender: sender.clone(),
            file_level,
            stdout_level,
        },
        LoggerGuard {
            sender,
            thread: Some(thread),
        },
    ))
}

impl LogDestinations {
    fn for_level(level: &Level, file_level: Level, stdout_level: Level) -> Self {
        Self {
            file: level <= &file_level,
            stdout: level <= &stdout_level,
        }
    }

    fn is_empty(self) -> bool {
        !self.file && !self.stdout
    }
}

impl ChannelLogWriter {
    pub(crate) fn new(sender: Sender<LogMessage>, destinations: LogDestinations) -> Self {
        Self {
            sender,
            destinations,
            line: Vec::new(),
        }
    }
}

impl Write for ChannelLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buffer);

        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ChannelLogWriter {
    fn drop(&mut self) {
        if self.line.is_empty() || self.destinations.is_empty() {
            return;
        }

        let message = LogMessage::Line {
            bytes: mem::take(&mut self.line),
            destinations: self.destinations,
        };

        let _ = self.sender.send(message);
    }
}

impl<'a> MakeWriter<'a> for ChannelLogMakeWriter {
    type Writer = ChannelLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ChannelLogWriter::new(self.sender.clone(), LogDestinations::default())
    }

    fn make_writer_for(&'a self, metadata: &Metadata<'_>) -> Self::Writer {
        ChannelLogWriter::new(
            self.sender.clone(),
            LogDestinations::for_level(metadata.level(), self.file_level, self.stdout_level),
        )
    }
}

impl LoggerGuard {
    #[cfg(test)]
    pub(crate) fn shutdown(mut self) -> io::Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> io::Result<()> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };

        let _ = self.sender.send(LogMessage::Shutdown);

        match thread.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::other("Logger thread panicked")),
        }
    }
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run_logger(
    mut file: impl Write,
    mut stdout: impl Write,
    receiver: Receiver<LogMessage>,
) -> io::Result<()> {
    while let Ok(message) = receiver.recv() {
        match message {
            LogMessage::Line {
                bytes,
                destinations,
            } => {
                if destinations.file {
                    write_without_ansi(&mut file, &bytes)?;
                }
                if destinations.stdout {
                    stdout.write_all(&bytes)?;
                }
            }
            LogMessage::Shutdown => break,
        }
    }

    file.flush()?;
    stdout.flush()
}

fn write_without_ansi(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let mut plain_start = 0;
    let mut cursor = 0;

    while cursor + 1 < bytes.len() {
        if bytes[cursor] != 0x1b || bytes[cursor + 1] != b'[' {
            cursor += 1;
            continue;
        }

        writer.write_all(&bytes[plain_start..cursor])?;
        cursor += 2;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            cursor += 1;
            if (0x40..=0x7e).contains(&byte) {
                break;
            }
        }
        plain_start = cursor;
    }

    writer.write_all(&bytes[plain_start..])
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        fs::{File, remove_file},
        io::Write,
        path::PathBuf,
        process,
        rc::Rc,
        sync::atomic::{AtomicU64, Ordering},
    };

    use flume::unbounded;
    use tracing::Level;

    use crate::app::logging::logger_thread::{
        ChannelLogWriter, LogDestinations, LogMessage, run_logger, start_logger,
    };

    static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

    struct RecordingWriter {
        name: &'static str,
        writes: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.writes.borrow_mut().push(self.name);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sends_fragmented_writes_as_one_log_line() {
        let (sender, receiver) = unbounded();
        let mut writer = ChannelLogWriter::new(
            sender,
            LogDestinations {
                file: true,
                stdout: true,
            },
        );

        writer
            .write_all(b"first fragment")
            .expect("channel writer should accept the first fragment");
        writer
            .write_all(b" second fragment\n")
            .expect("channel writer should accept the second fragment");
        drop(writer);

        let LogMessage::Line { bytes, .. } = receiver.recv().expect("log line should be sent")
        else {
            panic!("channel should contain a log line");
        };

        assert_eq!(bytes.as_slice(), b"first fragment second fragment\n");
    }

    #[test]
    fn retains_all_log_lines_until_the_logger_consumes_them() {
        let (sender, receiver) = unbounded();
        let destinations = LogDestinations {
            file: true,
            stdout: true,
        };
        let mut first = ChannelLogWriter::new(sender.clone(), destinations);
        first
            .write_all(b"first\n")
            .expect("channel writer should accept the first line");
        drop(first);

        let mut second = ChannelLogWriter::new(sender, destinations);
        second
            .write_all(b"second\n")
            .expect("channel writer should accept the second line");
        drop(second);

        let LogMessage::Line { bytes: first, .. } =
            receiver.recv().expect("first log line should be retained")
        else {
            panic!("channel should contain the first log line");
        };
        let LogMessage::Line { bytes: second, .. } =
            receiver.recv().expect("second log line should be retained")
        else {
            panic!("channel should contain the second log line");
        };

        assert_eq!(first.as_slice(), b"first\n");
        assert_eq!(second.as_slice(), b"second\n");
    }

    #[test]
    fn logger_writes_queued_lines_before_shutdown_returns() {
        let path = temp_log_path();
        let file = File::create(&path).expect("temporary log file should be created");
        let (make_writer, guard) =
            start_logger(file, Level::INFO, Level::INFO).expect("Logger thread should start");

        let mut first = ChannelLogWriter::new(
            make_writer.sender.clone(),
            LogDestinations {
                file: true,
                stdout: false,
            },
        );
        first
            .write_all(b"first\n")
            .expect("first log line should be buffered");
        drop(first);

        let mut second = ChannelLogWriter::new(
            make_writer.sender.clone(),
            LogDestinations {
                file: true,
                stdout: false,
            },
        );
        second
            .write_all(b"second\n")
            .expect("second log line should be buffered");
        drop(second);

        guard.shutdown().expect("Logger thread should stop");

        let contents = std::fs::read(&path).expect("temporary log file should be readable");
        remove_file(&path).expect("temporary log file should be removed");

        assert_eq!(contents.as_slice(), b"first\nsecond\n");
    }

    #[test]
    fn preserves_independent_file_and_stdout_levels() {
        assert_eq!(
            LogDestinations::for_level(&Level::TRACE, Level::INFO, Level::DEBUG),
            LogDestinations::default()
        );
        assert_eq!(
            LogDestinations::for_level(&Level::DEBUG, Level::INFO, Level::DEBUG),
            LogDestinations {
                file: false,
                stdout: true,
            }
        );
        assert_eq!(
            LogDestinations::for_level(&Level::INFO, Level::INFO, Level::DEBUG),
            LogDestinations {
                file: true,
                stdout: true,
            }
        );
    }

    #[test]
    fn logger_writes_the_file_before_stdout() {
        let (sender, receiver) = unbounded();
        sender
            .send(LogMessage::Line {
                bytes: b"both\n".to_vec(),
                destinations: LogDestinations {
                    file: true,
                    stdout: true,
                },
            })
            .expect("combined log line should be queued");
        sender
            .send(LogMessage::Shutdown)
            .expect("logger shutdown should be queued");

        let writes = Rc::new(RefCell::new(Vec::new()));
        let file = RecordingWriter {
            name: "file",
            writes: writes.clone(),
        };
        let stdout = RecordingWriter {
            name: "stdout",
            writes: writes.clone(),
        };
        run_logger(file, stdout, receiver).expect("Logger should drain queued output");

        assert_eq!(writes.borrow().as_slice(), ["file", "stdout"]);
    }

    #[test]
    fn logger_strips_ansi_from_file_but_preserves_stdout_color() {
        let colored = b"\x1b[2m2026-07-22\x1b[0m \x1b[32m INFO\x1b[0m message\n";
        let (sender, receiver) = unbounded();
        sender
            .send(LogMessage::Line {
                bytes: colored.to_vec(),
                destinations: LogDestinations {
                    file: true,
                    stdout: true,
                },
            })
            .expect("colored log line should be queued");
        sender
            .send(LogMessage::Shutdown)
            .expect("logger shutdown should be queued");

        let mut file = Vec::new();
        let mut stdout = Vec::new();
        run_logger(&mut file, &mut stdout, receiver).expect("Logger should drain queued output");

        assert_eq!(file, b"2026-07-22  INFO message\n");
        assert_eq!(stdout, colored);
    }

    fn temp_log_path() -> PathBuf {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!("rabbit-file-logger-{}-{id}.log", process::id(),))
    }
}
