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
        infra::platform::{gpu::GpuContext, screen_capture::kms::KmsCaptureLease},
        kernel::screen_capture::ScreenCaptureSource,
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn captures_one_composed_frame() {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .expect("RABBIT_KMS_SCREEN must name the DRM connector to capture");
        let ScreenCaptureSource { lease, receiver } =
            KmsCaptureLease::new(screen_name).expect("KMS capture source should start");
        let (device, frames) = receiver.into_parts();
        let device = device
            .recv()
            .expect("KMS capture worker should report its GPU")
            .expect("KMS capture GPU discovery should succeed");
        let mut frame = frames
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

        let context = GpuContext::new(&device).expect("Pipeline GPU context should initialize");
        let fence = frame
            .buffer
            .readiness_fence
            .take()
            .expect("Composed KMS frame should carry a readiness fence");
        context
            .egl()
            .wait_on_native_fence(fence)
            .expect("Pipeline GPU context should enqueue the source readiness wait");
        let image = context
            .egl()
            .import_dma_buf_frame(&frame.buffer)
            .expect("Pipeline GPU context should import the composed KMS frame");
        let _texture = context
            .egl()
            .create_dma_buf_texture(&image)
            .expect("Pipeline GPU context should bind the composed KMS frame as a texture");

        drop(lease);
    }
}
