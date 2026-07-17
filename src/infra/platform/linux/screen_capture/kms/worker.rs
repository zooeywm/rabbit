use std::{
    io,
    thread::{self, JoinHandle},
};

use eros::Context;
use flume::{Receiver, Sender, TryRecvError, TrySendError, bounded};

use crate::{
    infra::platform::screen_capture::kms::{
        capture::KmsCapturer,
        types::{DmaBufFrame, KmsPlaneIssue},
    },
    kernel::screen_capture::CapturedFrame,
};

type KmsCapturedFrame = CapturedFrame<DmaBufFrame, KmsPlaneIssue>;

enum KmsCaptureCommand {
    Start,
    Stop,
    Shutdown,
}

#[derive(Debug)]
pub(crate) struct KmsCaptureWorker {
    commands: Sender<KmsCaptureCommand>,
    frames: Receiver<eros::Result<KmsCapturedFrame>>,
    thread: Option<JoinHandle<()>>,
}

impl KmsCaptureWorker {
    pub(crate) fn new(screen_name: String) -> io::Result<Self> {
        let (commands, command_receiver) = bounded(2);
        let (frame_sender, frames) = bounded(1);
        let overflow_frames = frames.clone();
        let thread_name = format!("rabbit-kms-{screen_name}");
        let thread = thread::Builder::new().name(thread_name).spawn(move || {
            run_worker(screen_name, command_receiver, frame_sender, overflow_frames);
        })?;

        Ok(Self {
            commands,
            frames,
            thread: Some(thread),
        })
    }

    pub(crate) fn start(&self) -> eros::Result<()> {
        self.send_command(KmsCaptureCommand::Start, "starting capture")
    }

    pub(crate) fn stop(&self) -> eros::Result<()> {
        self.send_command(KmsCaptureCommand::Stop, "stopping capture")
    }

    pub(crate) async fn receive_frame(&self) -> eros::Result<KmsCapturedFrame> {
        let frame = self
            .frames
            .recv_async()
            .await
            .with_context(|| "KMS capture worker stopped before publishing a frame")?;

        frame
    }

    fn send_command(&self, command: KmsCaptureCommand, action: &str) -> eros::Result<()> {
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eros::bail!("KMS capture worker command queue is full while {}", action);
            }
            Err(TrySendError::Disconnected(_)) => {
                eros::bail!("KMS capture worker has stopped while {}", action);
            }
        }

        Ok(())
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

fn run_worker(
    screen_name: String,
    commands: Receiver<KmsCaptureCommand>,
    frames: Sender<eros::Result<KmsCapturedFrame>>,
    overflow_frames: Receiver<eros::Result<KmsCapturedFrame>>,
) {
    let mut capturer = None;

    while let Ok(command) = commands.recv() {
        match command {
            KmsCaptureCommand::Start => {
                if !run_capture_loop(
                    &screen_name,
                    &mut capturer,
                    &commands,
                    &frames,
                    &overflow_frames,
                ) {
                    return;
                }
            }
            KmsCaptureCommand::Stop => {}
            KmsCaptureCommand::Shutdown => return,
        }
    }
}

fn run_capture_loop(
    screen_name: &str,
    capturer: &mut Option<KmsCapturer>,
    commands: &Receiver<KmsCaptureCommand>,
    frames: &Sender<eros::Result<KmsCapturedFrame>>,
    overflow_frames: &Receiver<eros::Result<KmsCapturedFrame>>,
) -> bool {
    loop {
        match commands.try_recv() {
            Ok(KmsCaptureCommand::Start) | Err(TryRecvError::Empty) => {}
            Ok(KmsCaptureCommand::Stop) => return true,
            Ok(KmsCaptureCommand::Shutdown) | Err(TryRecvError::Disconnected) => return false,
        }

        let frame = capture_frame(screen_name, capturer);
        let capture_failed = frame.is_err();
        if !publish_latest(frames, overflow_frames, frame) {
            return false;
        }
        if capture_failed {
            return true;
        }
    }
}

fn publish_latest<T>(sender: &Sender<T>, receiver: &Receiver<T>, mut item: T) -> bool {
    loop {
        match sender.try_send(item) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned_item)) => {
                item = returned_item;
                match receiver.try_recv() {
                    Ok(_) | Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => return false,
                }
            }
            Err(TrySendError::Disconnected(_)) => return false,
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
    use flume::bounded;

    use crate::infra::platform::screen_capture::kms::worker::{KmsCaptureWorker, publish_latest};

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn worker_starts_without_opening_the_kms_output() -> std::io::Result<()> {
        let worker = KmsCaptureWorker::new("not-a-real-output".to_owned())?;

        drop(worker);

        Ok(())
    }

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn worker_keeps_only_the_latest_unconsumed_frame() {
        let (sender, receiver) = bounded(1);
        let overflow_receiver = receiver.clone();

        assert!(publish_latest(&sender, &overflow_receiver, 1));
        assert!(publish_latest(&sender, &overflow_receiver, 2));
        assert_eq!(receiver.try_recv(), Ok(2));
    }
}
