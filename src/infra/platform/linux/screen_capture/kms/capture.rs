use eros::Context;

use crate::{
    infra::platform::screen_capture::kms::{
        gbm_allocator::GbmFrameAllocator,
        output::KmsOutput,
        types::{DmaBufFrame, KmsPlaneIssue},
    },
    kernel::screen_capture::CapturedFrame,
};

#[derive(Debug)]
pub(crate) struct KmsCapturer {
    output: KmsOutput,
    allocator: GbmFrameAllocator,
}

impl KmsCapturer {
    pub(crate) fn new(screen_name: &str) -> eros::Result<Self> {
        let output = KmsOutput::open(screen_name)
            .with_context(|| format!("Failed to open KMS output {screen_name}"))?;
        let allocator = GbmFrameAllocator::new(&output.device)
            .with_context(|| format!("Failed to create KMS compositor for {screen_name}"))?;

        Ok(Self { output, allocator })
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
    use std::{future::poll_fn, pin::Pin};

    use eros::Context;
    use futures_core::Stream;

    use crate::infra::platform::screen_capture::kms::KmsCaptureSource;

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn captures_one_composed_frame() -> eros::Result<()> {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .with_context(|| "RABBIT_KMS_SCREEN must name the DRM connector to capture")?;
        let runtime = compio::runtime::Runtime::new()
            .with_context(|| "Failed to start the Compio test runtime")?;
        let frame = runtime.block_on(async move {
            let mut source = KmsCaptureSource::new(screen_name)
                .with_context(|| "Failed to start the KMS capture source")?;
            let mut subscription = source
                .subscribe()
                .with_context(|| "Failed to start the KMS capture subscription")?;

            let Some(frame) =
                poll_fn(|context| Pin::new(&mut subscription).poll_next(context)).await
            else {
                eros::bail!("KMS capture subscription closed before publishing a frame");
            };

            frame
        })?;

        for issue in &frame.issues {
            eprintln!("{issue}");
        }
        eprintln!("Captured KMS frame: {:#?}", frame.buffer);

        assert!(frame.buffer.size.width > 0);
        assert!(frame.buffer.size.height > 0);
        assert!(!frame.buffer.objects.is_empty());
        assert!(!frame.buffer.planes.is_empty());
        assert!(frame.buffer.readiness_fence.is_some());

        Ok(())
    }
}
