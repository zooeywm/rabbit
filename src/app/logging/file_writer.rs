use std::{
    fs::File,
    io::{self, Write},
    mem,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, unbounded};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FileLogMessage {
    Line(Vec<u8>),
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) struct ChannelFileMakeWriter {
    sender: Sender<FileLogMessage>,
}

#[derive(Debug)]
pub(crate) struct ChannelFileWriter {
    sender: Sender<FileLogMessage>,
    line: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct LoggerGuard {
    sender: Sender<FileLogMessage>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

pub(crate) fn start_logger(file: File) -> io::Result<(ChannelFileMakeWriter, LoggerGuard)> {
    let (sender, receiver) = unbounded();
    let thread = thread::Builder::new()
        .name("rabbit-logger".to_owned())
        .spawn(move || run_logger(file, receiver))?;

    Ok((
        ChannelFileMakeWriter {
            sender: sender.clone(),
        },
        LoggerGuard {
            sender,
            thread: Some(thread),
        },
    ))
}

impl ChannelFileMakeWriter {
    pub(crate) fn make_writer(&self) -> ChannelFileWriter {
        ChannelFileWriter::new(self.sender.clone())
    }
}

impl ChannelFileWriter {
    pub(crate) fn new(sender: Sender<FileLogMessage>) -> Self {
        Self {
            sender,
            line: Vec::new(),
        }
    }
}

impl Write for ChannelFileWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buffer);

        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ChannelFileWriter {
    fn drop(&mut self) {
        if self.line.is_empty() {
            return;
        }

        let message = FileLogMessage::Line(mem::take(&mut self.line));

        let _ = self.sender.send(message);
    }
}

impl LoggerGuard {
    pub(crate) fn shutdown(mut self) -> io::Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> io::Result<()> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };

        let _ = self.sender.send(FileLogMessage::Shutdown);

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

fn run_logger(mut file: File, receiver: Receiver<FileLogMessage>) -> io::Result<()> {
    while let Ok(message) = receiver.recv() {
        match message {
            FileLogMessage::Line(line) => file.write_all(&line)?,
            FileLogMessage::Shutdown => break,
        }
    }

    file.flush()
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{File, remove_file},
        io::Write,
        path::PathBuf,
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use flume::unbounded;

    use crate::app::logging::file_writer::{ChannelFileWriter, FileLogMessage, start_logger};

    static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn sends_fragmented_writes_as_one_log_line() {
        let (sender, receiver) = unbounded();
        let mut writer = ChannelFileWriter::new(sender);

        writer
            .write_all(b"first fragment")
            .expect("channel writer should accept the first fragment");
        writer
            .write_all(b" second fragment\n")
            .expect("channel writer should accept the second fragment");
        drop(writer);

        let FileLogMessage::Line(line) = receiver.recv().expect("log line should be sent") else {
            panic!("channel should contain a log line");
        };

        assert_eq!(line.as_slice(), b"first fragment second fragment\n");
    }

    #[test]
    fn retains_all_log_lines_until_the_logger_consumes_them() {
        let (sender, receiver) = unbounded();
        let mut first = ChannelFileWriter::new(sender.clone());
        first
            .write_all(b"first\n")
            .expect("channel writer should accept the first line");
        drop(first);

        let mut second = ChannelFileWriter::new(sender);
        second
            .write_all(b"second\n")
            .expect("channel writer should accept the second line");
        drop(second);

        let FileLogMessage::Line(first) =
            receiver.recv().expect("first log line should be retained")
        else {
            panic!("channel should contain the first log line");
        };
        let FileLogMessage::Line(second) =
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
        let (make_writer, guard) = start_logger(file).expect("Logger thread should start");

        let mut first = make_writer.make_writer();
        first
            .write_all(b"first\n")
            .expect("first log line should be buffered");
        drop(first);

        let mut second = make_writer.make_writer();
        second
            .write_all(b"second\n")
            .expect("second log line should be buffered");
        drop(second);

        guard.shutdown().expect("Logger thread should stop");

        let contents = std::fs::read(&path).expect("temporary log file should be readable");
        remove_file(&path).expect("temporary log file should be removed");

        assert_eq!(contents.as_slice(), b"first\nsecond\n");
    }

    fn temp_log_path() -> PathBuf {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!("rabbit-file-logger-{}-{id}.log", process::id(),))
    }
}
