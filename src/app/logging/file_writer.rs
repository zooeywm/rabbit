use std::{
    io::{self, Write},
    mem,
    sync::mpsc::{SyncSender, TrySendError},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FileLogMessage {
    Line(Vec<u8>),
}

#[derive(Debug)]
pub(crate) struct ChannelFileWriter {
    sender: SyncSender<FileLogMessage>,
    line: Vec<u8>,
}

impl ChannelFileWriter {
    pub(crate) fn new(sender: SyncSender<FileLogMessage>) -> Self {
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

        match self.sender.try_send(message) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::mpsc::{TryRecvError, sync_channel},
    };

    use crate::app::logging::file_writer::{ChannelFileWriter, FileLogMessage};

    #[test]
    fn sends_fragmented_writes_as_one_log_line() {
        let (sender, receiver) = sync_channel(1);
        let mut writer = ChannelFileWriter::new(sender);

        writer
            .write_all(b"first fragment")
            .expect("channel writer should accept the first fragment");
        writer
            .write_all(b" second fragment\n")
            .expect("channel writer should accept the second fragment");
        drop(writer);

        let FileLogMessage::Line(line) = receiver.recv().expect("log line should be sent");

        assert_eq!(line.as_slice(), b"first fragment second fragment\n");
    }

    #[test]
    fn drops_the_current_log_line_when_the_channel_is_full() {
        let (sender, receiver) = sync_channel(1);
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

        let FileLogMessage::Line(line) = receiver
            .recv()
            .expect("first log line should be retained");

        assert_eq!(line.as_slice(), b"first\n");
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
    }
}
