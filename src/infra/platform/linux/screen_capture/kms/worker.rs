use std::{
    io,
    thread::{self, JoinHandle},
};

use eros::Context;
use flume::{Receiver, Sender, TrySendError, bounded};
use futures_channel::oneshot;

use crate::{
    infra::platform::screen_capture::kms::{
        capture::KmsCapturer,
        types::{DmaBufFrame, KmsPlaneIssue},
    },
    kernel::screen_capture::CapturedFrame,
};

type KmsCapturedFrame = CapturedFrame<DmaBufFrame, KmsPlaneIssue>;

enum KmsCaptureCommand {
    Capture(oneshot::Sender<eros::Result<KmsCapturedFrame>>),
    Shutdown,
}

#[derive(Debug)]
pub(crate) struct KmsCaptureWorker {
    commands: Sender<KmsCaptureCommand>,
    thread: Option<JoinHandle<()>>,
}

impl KmsCaptureWorker {
    pub(crate) fn new(screen_name: String) -> io::Result<Self> {
        let (commands, receiver) = bounded(1);
        let thread_name = format!("rabbit-kms-{screen_name}");
        let thread = thread::Builder::new()
            .name(thread_name)
            .spawn(move || run_worker(screen_name, receiver))?;

        Ok(Self {
            commands,
            thread: Some(thread),
        })
    }

    pub(crate) async fn capture(&self) -> eros::Result<KmsCapturedFrame> {
        let (result_sender, result_receiver) = oneshot::channel();

        match self
            .commands
            .try_send(KmsCaptureCommand::Capture(result_sender))
        {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eros::bail!("KMS capture worker already has a queued capture request");
            }
            Err(TrySendError::Disconnected(_)) => {
                eros::bail!("KMS capture worker has stopped");
            }
        }

        result_receiver
            .await
            .with_context(|| "KMS capture worker stopped before returning a frame")?
    }
}

impl Drop for KmsCaptureWorker {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };

        let _ = self.commands.send(KmsCaptureCommand::Shutdown);
        let _ = thread.join();
    }
}

fn run_worker(screen_name: String, receiver: Receiver<KmsCaptureCommand>) {
    let mut capturer = None;

    while let Ok(command) = receiver.recv() {
        match command {
            KmsCaptureCommand::Capture(result_sender) => {
                let _ = result_sender.send(capture_frame(&screen_name, &mut capturer));
            }
            KmsCaptureCommand::Shutdown => return,
        }
    }
}

fn capture_frame(
    screen_name: &str,
    capturer: &mut Option<KmsCapturer>,
) -> eros::Result<KmsCapturedFrame> {
    if let Some(capturer) = capturer.as_ref() {
        return capturer.capture();
    }

    let capturer = capturer.insert(KmsCapturer::new(screen_name)?);
    capturer.capture()
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::screen_capture::kms::worker::KmsCaptureWorker;

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn worker_starts_without_opening_the_kms_output() {
        let worker = KmsCaptureWorker::new("not-a-real-output".to_owned())
            .expect("KMS worker thread should start without opening the output");

        drop(worker);
    }
}
