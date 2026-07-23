#[cfg(test)]
use std::path::PathBuf;
use std::{
    io,
    thread::{self, JoinHandle},
    time::Duration,
};

use flume::{Receiver, Sender, TryRecvError, TrySendError, bounded};

use crate::{
    infra::WorkerReaperHandle,
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufProfile},
        gpu::GpuDevice,
        screen_capture::kms::{
            capture::{KmsCaptureOutput, KmsCapturer},
            types::{KmsFramebufferPlane, KmsPlaneIssue},
        },
        video_probe::{VideoFrameProbe, VideoProbeClock},
    },
    kernel::{geometry::FrameRate, screen_capture::ScreenCaptureSource},
};

#[derive(Debug)]
pub(crate) struct KmsCapturedFrame {
    pub(crate) source: KmsCapturedSource,
    pub(crate) issues: Vec<KmsPlaneIssue>,
    pub(crate) frame_rate: FrameRate,
    pub(crate) probe: Option<VideoFrameProbe>,
}

#[derive(Debug)]
pub(crate) enum KmsCapturedSource {
    PlaneSet {
        output_size: crate::kernel::geometry::PixelSize,
        planes: Vec<KmsFramebufferPlane>,
    },
    Composed(DmaBufFrame),
}

#[cfg(test)]
pub(crate) fn empty_kms_frame(size: crate::kernel::geometry::PixelSize) -> KmsCapturedFrame {
    KmsCapturedFrame {
        source: KmsCapturedSource::Composed(DmaBufFrame {
            size,
            format: drm::buffer::DrmFourcc::Xrgb8888,
            objects: Vec::new(),
            planes: Vec::new(),
            readiness_fence: None,
            lease: None,
            va_backing: None,
        }),
        issues: Vec::new(),
        frame_rate: FrameRate::new(60, 1).expect("Test frame rate should be valid"),
        probe: None,
    }
}

enum KmsCaptureCommand {
    UseComposedFallback,
    Shutdown,
}

struct KmsCaptureLoop {
    screen_name: String,
    command_receiver: Receiver<KmsCaptureCommand>,
    device_sender: Sender<eros::Result<GpuDevice>>,
    frame_sender: Sender<eros::Result<KmsCapturedFrame>>,
    overflow_frames: Receiver<eros::Result<KmsCapturedFrame>>,
    enable_probing: bool,
    probe_interval: Duration,
    encoder_profiles: Vec<DmaBufProfile>,
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
    commands: Sender<KmsCaptureCommand>,
}

#[derive(Debug, Clone)]
pub(crate) struct KmsCompositionFallback {
    commands: Sender<KmsCaptureCommand>,
}

impl KmsCaptureLease {
    pub(crate) fn new(
        screen_name: String,
        enable_probing: bool,
        probe_interval: Duration,
        reaper: WorkerReaperHandle,
        encoder_profiles: Vec<DmaBufProfile>,
    ) -> io::Result<ScreenCaptureSource<Self, KmsFrameReceiver>> {
        let (commands, command_receiver) = bounded(1);
        let (device_sender, device) = bounded(1);
        let (frame_sender, frames) = bounded(1);
        let overflow_frames = frames.clone();
        let thread_name = format!("rabbit-kms-{screen_name}");
        let thread = thread::Builder::new().name(thread_name).spawn(move || {
            KmsCaptureLoop {
                screen_name,
                command_receiver,
                device_sender,
                frame_sender,
                overflow_frames,
                enable_probing,
                probe_interval,
                encoder_profiles,
            }
            .run();
        })?;

        Ok(ScreenCaptureSource {
            lease: Self {
                commands: commands.clone(),
                thread: Some(thread),
                reaper: Some(reaper),
            },
            receiver: KmsFrameReceiver {
                device,
                frames,
                commands: commands.clone(),
            },
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
        KmsCompositionFallback,
    ) {
        (
            self.device,
            self.frames,
            KmsCompositionFallback {
                commands: self.commands,
            },
        )
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
        let (commands, _) = bounded(1);
        device_sender
            .send(Ok(gpu_device))
            .expect("Test GPU device should be sent");

        (
            sender,
            Self {
                device,
                frames,
                commands,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let (_, receiver) = Self::channel();

        receiver
    }
}

impl KmsCompositionFallback {
    pub(crate) fn request(&self) {
        let _ = self
            .commands
            .try_send(KmsCaptureCommand::UseComposedFallback);
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

impl KmsCaptureLoop {
    fn run(self) {
        let Self {
            screen_name,
            command_receiver: commands,
            device_sender: device,
            frame_sender: frames,
            overflow_frames,
            enable_probing,
            probe_interval,
            encoder_profiles,
        } = self;

        let mut capturer = match KmsCapturer::new(&screen_name, encoder_profiles) {
            Ok(capturer) => capturer,
            Err(error) => {
                let _ = device.send(Err(error));
                return;
            }
        };

        if device.send(Ok(capturer.gpu_device().clone())).is_err() {
            return;
        }

        let mut probe_clock = enable_probing.then(|| VideoProbeClock::new(probe_interval));
        let mut use_composed_fallback = false;

        loop {
            loop {
                match commands.try_recv() {
                    Ok(KmsCaptureCommand::UseComposedFallback) => {
                        use_composed_fallback = true;
                    }
                    Ok(KmsCaptureCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }

            let frame = if let Some(clock) = &mut probe_clock {
                match capturer.capture_with_timing(use_composed_fallback) {
                    Ok((Some(frame), timing)) => {
                        Ok(Some(captured_frame(frame, Some(clock.frame(timing)))))
                    }
                    Ok((None, _)) => Ok(None),
                    Err(error) => Err(error),
                }
            } else {
                capturer
                    .capture(use_composed_fallback)
                    .map(|frame| frame.map(|frame| captured_frame(frame, None)))
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
}

fn captured_frame(frame: KmsCaptureOutput, probe: Option<VideoFrameProbe>) -> KmsCapturedFrame {
    match frame {
        KmsCaptureOutput::PlaneSet(frame) => KmsCapturedFrame {
            source: KmsCapturedSource::PlaneSet {
                output_size: frame.output_size,
                planes: frame.planes,
            },
            issues: frame.issues,
            frame_rate: frame.frame_rate,
            probe,
        },
        KmsCaptureOutput::Composed(frame) => KmsCapturedFrame {
            source: KmsCapturedSource::Composed(frame.buffer),
            issues: frame.issues,
            frame_rate: frame.frame_rate,
            probe,
        },
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
                        if let KmsCapturedSource::Composed(buffer) = &mut frame.source
                            && let Some(fence) = buffer.readiness_fence.take()
                        {
                            buffer.set_release_fence(fence);
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
    use std::time::Duration;

    use flume::bounded;

    use crate::{
        infra::platform::screen_capture::kms::worker::{
            KmsCaptureCommand, KmsCaptureLease, KmsCapturedSource, KmsCompositionFallback,
            empty_kms_frame, publish_latest_frame,
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
            Duration::from_secs(2),
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
        let KmsCapturedSource::Composed(buffer) = frame.source else {
            panic!("Test frame should use the composed source variant");
        };
        assert_eq!(buffer.size, latest_size);
    }

    #[test]
    fn gpu_worker_can_request_the_composed_fallback() {
        let (commands, receiver) = bounded(1);
        let fallback = KmsCompositionFallback { commands };

        fallback.request();

        assert!(matches!(
            receiver
                .try_recv()
                .expect("Fallback command should be queued"),
            KmsCaptureCommand::UseComposedFallback
        ));
    }
}
