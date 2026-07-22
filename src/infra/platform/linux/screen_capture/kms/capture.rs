use std::time::Instant;

use eros::Context;

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufProfile},
        gpu::GpuDevice,
        screen_capture::kms::{
            gbm_allocator::GbmFrameAllocator,
            output::KmsOutput,
            types::{KmsFramebufferSnapshot, KmsPlaneIssue},
        },
        video_probe::VideoCaptureTiming,
    },
    kernel::screen_capture::CapturedFrame,
};

#[derive(Debug)]
pub(crate) struct KmsCapturer {
    output: KmsOutput,
    allocator: GbmFrameAllocator,
    gpu_device: GpuDevice,
}

pub(super) enum KmsCaptureOutput {
    PlaneSet(KmsFramebufferSnapshot),
    Composed(CapturedFrame<DmaBufFrame, KmsPlaneIssue>),
}

impl KmsCapturer {
    pub(crate) fn new(
        screen_name: &str,
        encoder_profiles: Vec<DmaBufProfile>,
    ) -> eros::Result<Self> {
        let output = KmsOutput::open(screen_name)
            .with_context(|| format!("Failed to open KMS output {screen_name}"))?;
        let gpu_device = GpuDevice::from(output.device.render_node_path()?);
        let allocator = GbmFrameAllocator::new(&output.device, encoder_profiles)
            .with_context(|| format!("Failed to create KMS compositor for {screen_name}"))?;

        Ok(Self {
            output,
            allocator,
            gpu_device,
        })
    }

    pub(crate) fn gpu_device(&self) -> &GpuDevice {
        &self.gpu_device
    }

    pub(crate) fn capture(
        &mut self,
        use_composed_fallback: bool,
    ) -> eros::Result<Option<KmsCaptureOutput>> {
        self.wait_for_vblank()?;
        self.capture_current_frame(use_composed_fallback)
    }

    fn wait_for_vblank(&self) -> eros::Result<()> {
        self.output
            .wait_for_vblank()
            .with_context(|| "Failed to synchronize KMS capture with the display refresh")?;

        Ok(())
    }

    fn capture_current_frame(
        &mut self,
        use_composed_fallback: bool,
    ) -> eros::Result<Option<KmsCaptureOutput>> {
        let Some(snapshot) = self
            .output
            .snapshot_framebuffers()
            .with_context(|| "Failed to snapshot KMS framebuffers")?
        else {
            return Ok(None);
        };

        if !use_composed_fallback {
            return Ok(Some(KmsCaptureOutput::PlaneSet(snapshot)));
        }

        self.allocator
            .compose(snapshot)
            .with_context(|| "Failed to compose KMS framebuffers")
            .map(|frame| frame.map(KmsCaptureOutput::Composed))
    }

    pub(crate) fn capture_with_timing(
        &mut self,
        use_composed_fallback: bool,
    ) -> eros::Result<(Option<KmsCaptureOutput>, VideoCaptureTiming)> {
        let vblank_wait_started = Instant::now();
        self.wait_for_vblank()?;
        let capture_started = Instant::now();
        let frame = self.capture_current_frame(use_composed_fallback)?;
        let capture_completed = Instant::now();

        Ok((
            frame,
            VideoCaptureTiming {
                vblank_wait_started,
                capture_started,
                capture_completed,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        infra::platform::{gpu::GpuContext, screen_capture::kms::KmsCaptureLease},
        kernel::screen_capture::ScreenCaptureSource,
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn captures_one_raw_plane_set() {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .expect("RABBIT_KMS_SCREEN must name the DRM connector to capture");
        let (_reaper, reaper_handle) =
            crate::infra::WorkerReaper::new().expect("Test worker reaper should start");
        let ScreenCaptureSource { lease, receiver } = KmsCaptureLease::new(
            screen_name,
            false,
            Duration::from_secs(2),
            reaper_handle,
            Vec::new(),
        )
        .expect("KMS capture source should start");
        let (device, frames, _fallback) = receiver.into_parts();
        let device = device
            .recv()
            .expect("KMS capture worker should report its GPU")
            .expect("KMS capture GPU discovery should succeed");
        let frame = frames
            .recv()
            .expect("KMS capture worker should remain connected")
            .expect("KMS capture worker should publish one frame");

        for issue in &frame.issues {
            eprintln!("{issue}");
        }
        let crate::infra::platform::screen_capture::KmsCapturedSource::PlaneSet {
            output_size,
            planes,
        } = frame.source
        else {
            panic!("KMS capture should publish the raw plane-set fast path");
        };
        eprintln!("Captured KMS plane set: {planes:#?}");

        assert!(output_size.width > 0);
        assert!(output_size.height > 0);
        assert!(!planes.is_empty());
        assert!(planes.iter().all(|plane| !plane.buffer.objects.is_empty()));

        let _context = GpuContext::new(&device).expect("Pipeline GPU context should initialize");

        drop(lease);
    }
}
