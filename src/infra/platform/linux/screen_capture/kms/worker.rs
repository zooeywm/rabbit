#[cfg(test)]
use std::path::PathBuf;
use std::{
    io,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, TryRecvError, TrySendError, bounded};

use crate::{
    infra::platform::{
        dma_buf::DmaBufFrame,
        gpu::GpuDevice,
        screen_capture::kms::{capture::KmsCapturer, types::KmsPlaneIssue},
        video_probe::{VideoFrameProbe, VideoProbeClock},
    },
    kernel::screen_capture::ScreenCaptureSource,
};

#[derive(Debug)]
pub(crate) struct KmsCapturedFrame {
    pub(crate) buffer: DmaBufFrame,
    pub(crate) issues: Vec<KmsPlaneIssue>,
    pub(crate) probe: Option<VideoFrameProbe>,
}

#[cfg(test)]
pub(crate) fn empty_kms_frame(size: crate::kernel::geometry::PixelSize) -> KmsCapturedFrame {
    KmsCapturedFrame {
        buffer: DmaBufFrame {
            size,
            format: drm::buffer::DrmFourcc::Xrgb8888,
            objects: Vec::new(),
            planes: Vec::new(),
            readiness_fence: None,
        },
        issues: Vec::new(),
        probe: None,
    }
}

enum KmsCaptureCommand {
    Shutdown,
}

#[derive(Debug)]
pub(crate) struct KmsCaptureLease {
    commands: Sender<KmsCaptureCommand>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub(crate) struct KmsFrameReceiver {
    device: Receiver<eros::Result<GpuDevice>>,
    frames: Receiver<eros::Result<KmsCapturedFrame>>,
}

impl KmsCaptureLease {
    pub(crate) fn new(
        screen_name: String,
        enable_probing: bool,
    ) -> io::Result<ScreenCaptureSource<Self, KmsFrameReceiver>> {
        let (commands, command_receiver) = bounded(1);
        let (device_sender, device) = bounded(1);
        let (frame_sender, frames) = bounded(1);
        let overflow_frames = frames.clone();
        let thread_name = format!("rabbit-kms-{screen_name}");
        let thread = thread::Builder::new().name(thread_name).spawn(move || {
            run_capture_loop(
                screen_name,
                command_receiver,
                device_sender,
                frame_sender,
                overflow_frames,
                enable_probing,
            );
        })?;

        Ok(ScreenCaptureSource {
            lease: Self {
                commands,
                thread: Some(thread),
            },
            receiver: KmsFrameReceiver { device, frames },
        })
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let (commands, _) = bounded(1);

        Self {
            commands,
            thread: None,
        }
    }
}

impl KmsFrameReceiver {
    pub(crate) fn into_parts(
        self,
    ) -> (
        Receiver<eros::Result<GpuDevice>>,
        Receiver<eros::Result<KmsCapturedFrame>>,
    ) {
        (self.device, self.frames)
    }

    #[cfg(test)]
    pub(crate) fn channel() -> (Sender<eros::Result<KmsCapturedFrame>>, Self) {
        Self::channel_on(GpuDevice::from(PathBuf::from("/dev/dri/renderD128")))
    }

    #[cfg(test)]
    pub(crate) fn channel_on(
        gpu_device: GpuDevice,
    ) -> (Sender<eros::Result<KmsCapturedFrame>>, Self) {
        let (device_sender, device) = bounded(1);
        let (sender, frames) = bounded(1);
        device_sender
            .send(Ok(gpu_device))
            .expect("Test GPU device should be sent");

        (sender, Self { device, frames })
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let (_, receiver) = Self::channel();

        receiver
    }
}

impl Drop for KmsCaptureLease {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };

        let _ = self.commands.send(KmsCaptureCommand::Shutdown);
        let _ = thread.join();
    }
}

fn run_capture_loop(
    screen_name: String,
    commands: Receiver<KmsCaptureCommand>,
    device: Sender<eros::Result<GpuDevice>>,
    frames: Sender<eros::Result<KmsCapturedFrame>>,
    overflow_frames: Receiver<eros::Result<KmsCapturedFrame>>,
    enable_probing: bool,
) {
    let capturer = match KmsCapturer::new(&screen_name) {
        Ok(capturer) => capturer,
        Err(error) => {
            let _ = device.send(Err(error));
            return;
        }
    };

    if device.send(Ok(capturer.gpu_device().clone())).is_err() {
        return;
    }

    let mut probe_clock = enable_probing.then(VideoProbeClock::new);

    loop {
        match commands.try_recv() {
            Ok(KmsCaptureCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {}
        }

        let frame = if let Some(clock) = &mut probe_clock {
            capturer
                .capture_with_timing()
                .map(|(frame, timing)| KmsCapturedFrame {
                    buffer: frame.buffer,
                    issues: frame.issues,
                    probe: Some(clock.frame(timing)),
                })
        } else {
            capturer.capture().map(|frame| KmsCapturedFrame {
                buffer: frame.buffer,
                issues: frame.issues,
                probe: None,
            })
        };
        let capture_failed = frame.is_err();

        if !publish_latest(&frames, &overflow_frames, frame) || capture_failed {
            return;
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

#[cfg(test)]
mod tests {
    use flume::bounded;

    use crate::infra::platform::screen_capture::kms::worker::{KmsCaptureLease, publish_latest};

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn lease_starts_without_opening_the_kms_output_on_the_main_thread() {
        let source = KmsCaptureLease::new("not-a-real-output".to_owned(), false)
            .expect("KMS capture source should start asynchronously");

        drop(source);
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
