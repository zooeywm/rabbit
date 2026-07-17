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
    use crate::infra::platform::screen_capture::kms::worker::KmsCaptureWorker;

    #[test]
    #[ignore = "requires a real KMS output and CAP_SYS_ADMIN"]
    fn captures_one_composed_frame() {
        let screen_name = std::env::var("RABBIT_KMS_SCREEN")
            .expect("RABBIT_KMS_SCREEN must name the DRM connector to capture");
        let worker = KmsCaptureWorker::new(screen_name).expect("KMS worker thread should start");
        let runtime = compio::runtime::Runtime::new().expect("Compio runtime should start");
        let frame = runtime
            .block_on(worker.capture())
            .expect("KMS worker should capture one frame");

        for issue in &frame.issues {
            eprintln!("{issue}");
        }
        eprintln!("Captured KMS frame: {:#?}", frame.buffer);

        assert!(frame.buffer.size.width > 0);
        assert!(frame.buffer.size.height > 0);
        assert!(!frame.buffer.objects.is_empty());
        assert!(!frame.buffer.planes.is_empty());
    }
}
