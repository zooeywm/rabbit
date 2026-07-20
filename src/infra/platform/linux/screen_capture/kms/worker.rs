#[cfg(test)]
use std::path::PathBuf;
use std::{
    io,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, TryRecvError, TrySendError, bounded};

use crate::{
    infra::WorkerReaperHandle,
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
            lease: None,
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
    reaper: Option<WorkerReaperHandle>,
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
        reaper: WorkerReaperHandle,
        composition_modifiers: Vec<drm::buffer::DrmModifier>,
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
                composition_modifiers,
            );
        })?;

        Ok(ScreenCaptureSource {
            lease: Self {
                commands,
                thread: Some(thread),
                reaper: Some(reaper),
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
            reaper: None,
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

        let _ = self.commands.try_send(KmsCaptureCommand::Shutdown);
        if let Some(reaper) = &self.reaper {
            reaper.reap(thread);
        }
    }
}

fn run_capture_loop(
    screen_name: String,
    commands: Receiver<KmsCaptureCommand>,
    device: Sender<eros::Result<GpuDevice>>,
    frames: Sender<eros::Result<KmsCapturedFrame>>,
    overflow_frames: Receiver<eros::Result<KmsCapturedFrame>>,
    enable_probing: bool,
    composition_modifiers: Vec<drm::buffer::DrmModifier>,
) {
    let mut capturer = match KmsCapturer::new(&screen_name, composition_modifiers) {
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
            match capturer.capture_with_timing() {
                Ok((Some(frame), timing)) => Ok(Some(KmsCapturedFrame {
                    buffer: frame.buffer,
                    issues: frame.issues,
                    probe: Some(clock.frame(timing)),
                })),
                Ok((None, _)) => Ok(None),
                Err(error) => Err(error),
            }
        } else {
            capturer.capture().map(|frame| {
                frame.map(|frame| KmsCapturedFrame {
                    buffer: frame.buffer,
                    issues: frame.issues,
                    probe: None,
                })
            })
        };
        let capture_failed = frame.is_err();

        let frame = match frame {
            Ok(Some(frame)) => Ok(frame),
            Ok(None) => continue,
            Err(error) => Err(error),
        };

        if !publish_latest_frame(&frames, &overflow_frames, frame) || capture_failed {
            return;
        }
    }
}

fn publish_latest_frame(
    sender: &Sender<eros::Result<KmsCapturedFrame>>,
    receiver: &Receiver<eros::Result<KmsCapturedFrame>>,
    mut item: eros::Result<KmsCapturedFrame>,
) -> bool {
    loop {
        match sender.try_send(item) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned_item)) => {
                item = returned_item;
                match receiver.try_recv() {
                    Ok(Ok(mut frame)) => {
                        if let Some(fence) = frame.buffer.readiness_fence.take() {
                            frame.buffer.set_release_fence(fence);
                        }
                    }
                    Ok(Err(_)) | Err(TryRecvError::Empty) => {}
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

    use crate::{
        infra::platform::screen_capture::kms::worker::{
            KmsCaptureLease, empty_kms_frame, publish_latest_frame,
        },
        kernel::geometry::PixelSize,
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn lease_starts_without_opening_the_kms_output_on_the_main_thread() {
        let (_reaper, reaper_handle) =
            crate::infra::WorkerReaper::new().expect("Test worker reaper should start");
        let source = KmsCaptureLease::new(
            "not-a-real-output".to_owned(),
            false,
            reaper_handle,
            Vec::new(),
        )
        .expect("KMS capture source should start asynchronously");

        drop(source);
    }

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn worker_keeps_only_the_latest_unconsumed_frame() {
        let (sender, receiver) = bounded(1);
        let overflow_receiver = receiver.clone();
        let first_size = PixelSize {
            width: 1280,
            height: 720,
        };
        let latest_size = PixelSize {
            width: 1920,
            height: 1080,
        };

        assert!(publish_latest_frame(
            &sender,
            &overflow_receiver,
            Ok(empty_kms_frame(first_size)),
        ));
        assert!(publish_latest_frame(
            &sender,
            &overflow_receiver,
            Ok(empty_kms_frame(latest_size)),
        ));
        let frame = receiver
            .try_recv()
            .expect("Latest KMS frame should remain queued")
            .expect("Latest KMS frame should be successful");
        assert_eq!(frame.buffer.size, latest_size);
    }
}
