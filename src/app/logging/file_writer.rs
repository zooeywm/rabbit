use std::io::{self, Write};

#[derive(Debug, Default)]
pub(crate) struct ChannelFileWriter;

impl Write for ChannelFileWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::app::logging::file_writer::ChannelFileWriter;

    #[test]
    fn empty_channel_writer_accepts_a_log_line() {
        let mut writer = ChannelFileWriter;

        writer
            .write_all(b"first fragment")
            .expect("empty channel writer should accept the first fragment");
        writer
            .write_all(b" second fragment\n")
            .expect("empty channel writer should accept the second fragment");
        writer
            .flush()
            .expect("empty channel writer should accept flush");
    }
}
