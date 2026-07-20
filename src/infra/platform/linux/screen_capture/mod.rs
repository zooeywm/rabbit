mod kms;

#[cfg(test)]
pub(crate) use kms::empty_kms_frame;
pub(crate) use kms::{
    EglContext, EglDmaBufImage, KmsCaptureLease, KmsCapturedFrame, KmsFrameReceiver,
    KmsScreenCaptureManager, KmsScreenCaptureManagerState,
};

/// Creates the screen-capture manager state selected for Linux.
pub(crate) fn create_screen_capture_manager_state(
    enable_probing: bool,
    worker_reaper: crate::infra::WorkerReaperHandle,
) -> KmsScreenCaptureManagerState {
    KmsScreenCaptureManagerState::new(enable_probing, worker_reaper)
}
