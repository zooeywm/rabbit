use std::time::Instant;

use eros::Context;

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufProfile},
        gpu::GpuDevice,
        screen_capture::kms::{
            gbm_allocator::GbmFrameAllocator, output::KmsOutput, types::KmsPlaneIssue,
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
    ) -> eros::Result<Option<CapturedFrame<DmaBufFrame, KmsPlaneIssue>>> {
        self.wait_for_vblank()?;
        self.capture_current_frame()
    }

    fn wait_for_vblank(&self) -> eros::Result<()> {
        self.output
            .wait_for_vblank()
            .with_context(|| "Failed to synchronize KMS capture with the display refresh")?;

        Ok(())
    }

    fn capture_current_frame(
        &mut self,
    ) -> eros::Result<Option<CapturedFrame<DmaBufFrame, KmsPlaneIssue>>> {
        let snapshot = self
            .output
            .snapshot_framebuffers()
            .with_context(|| "Failed to snapshot KMS framebuffers")?;

        self.allocator
            .compose(snapshot)
            .with_context(|| "Failed to compose KMS framebuffers")
    }

    pub(crate) fn capture_with_timing(
        &mut self,
    ) -> eros::Result<(
        Option<CapturedFrame<DmaBufFrame, KmsPlaneIssue>>,
        VideoCaptureTiming,
    )> {
        let vblank_wait_started = Instant::now();
        self.wait_for_vblank()?;
        let capture_started = Instant::now();
        let frame = self.capture_current_frame()?;
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

    use crate::{
        infra::platform::{gpu::GpuContext, screen_capture::kms::KmsCaptureLease},
        kernel::screen_capture::ScreenCaptureSource,
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn captures_one_composed_frame() {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .expect("RABBIT_KMS_SCREEN must name the DRM connector to capture");
        let (_reaper, reaper_handle) =
            crate::infra::WorkerReaper::new().expect("Test worker reaper should start");
        let ScreenCaptureSource { lease, receiver } =
            KmsCaptureLease::new(screen_name, false, reaper_handle, Vec::new())
                .expect("KMS capture source should start");
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
        let texture = context
            .egl()
            .create_dma_buf_texture(&image)
            .expect("Pipeline GPU context should bind the composed KMS frame as a texture");
        let (mut output, _) = context
            .select_nv12_output(frame.buffer.size)
            .expect("Pipeline GPU context should allocate an NV12 output");
        let output_image = context
            .egl()
            .import_nv12_target(&output)
            .expect("Pipeline GPU context should import the NV12 output");
        let output_target = context
            .egl()
            .create_nv12_target(&output_image)
            .expect("Pipeline GPU context should bind the NV12 output targets");
        context
            .egl()
            .convert_to_nv12(&texture, &output_target)
            .expect("Pipeline GPU context should convert the composed frame to NV12");
        output.readiness_fence = Some(
            context
                .egl()
                .finish_frame_pipeline()
                .expect("Pipeline GPU context should export NV12 output readiness"),
        );

        assert!(output.readiness_fence.is_some());

        drop(lease);
    }
}
