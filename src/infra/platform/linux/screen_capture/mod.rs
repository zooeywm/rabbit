mod kms;

#[cfg(test)]
pub(crate) use kms::empty_kms_frame;
pub(crate) use kms::{
    KmsCaptureLease, KmsCapturedFrame, KmsFrameReceiver, KmsScreenCaptureManager,
    KmsScreenCaptureManagerState,
};

/// Creates the screen-capture manager state selected for Linux.
pub(crate) fn create_screen_capture_manager_state() -> KmsScreenCaptureManagerState {
    KmsScreenCaptureManagerState::new()
}
