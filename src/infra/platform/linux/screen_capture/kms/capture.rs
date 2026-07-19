use eros::Context;

use crate::{
    infra::platform::{
        dma_buf::DmaBufFrame,
        gpu::GpuDevice,
        screen_capture::kms::{
            gbm_allocator::GbmFrameAllocator, output::KmsOutput, types::KmsPlaneIssue,
        },
    },
    kernel::screen_capture::CapturedFrame,
};

#[derive(Debug)]
pub(crate) struct KmsCapturer {
    output: KmsOutput,
    allocator: GbmFrameAllocator,
    gpu_device: GpuDevice,
}

impl KmsCapturer {
    pub(crate) fn new(screen_name: &str) -> eros::Result<Self> {
        let output = KmsOutput::open(screen_name)
            .with_context(|| format!("Failed to open KMS output {screen_name}"))?;
        let gpu_device = GpuDevice::from(output.device.render_node_path()?);
        let allocator = GbmFrameAllocator::new(&output.device)
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

    pub(crate) fn capture(&self) -> eros::Result<CapturedFrame<DmaBufFrame, KmsPlaneIssue>> {
        self.output
            .wait_for_vblank()
            .with_context(|| "Failed to synchronize KMS capture with the display refresh")?;
        let snapshot = self
            .output
            .snapshot_framebuffers()
            .with_context(|| "Failed to snapshot KMS framebuffers")?;

        self.allocator
            .compose(snapshot)
            .with_context(|| "Failed to compose KMS framebuffers")
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::platform::screen_capture::kms::KmsCaptureLease,
        kernel::screen_capture::ScreenCaptureSource,
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn captures_one_composed_frame() {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .expect("RABBIT_KMS_SCREEN must name the DRM connector to capture");
        let ScreenCaptureSource { lease, receiver } =
            KmsCaptureLease::new(screen_name).expect("KMS capture source should start");
        let (_device, frames) = receiver.into_parts();
        let frame = frames
            .recv()
            .expect("KMS capture worker should remain connected")
            .expect("KMS capture worker should publish one frame");

        for issue in &frame.issues {
            eprintln!("{issue}");
        }
        eprintln!("Captured KMS frame: {:#?}", frame.buffer);

        assert!(frame.buffer.size.width > 0);
        assert!(frame.buffer.size.height > 0);
        assert!(!frame.buffer.objects.is_empty());
        assert!(frame.buffer.objects.iter().all(|object| object.size > 0));
        assert!(!frame.buffer.planes.is_empty());
        assert!(frame.buffer.readiness_fence.is_some());

        drop(lease);
    }
}
