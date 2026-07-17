mod kms;

pub(crate) use kms::KmsScreenCaptureManagerState;

/// Creates the screen-capture manager state selected for Linux.
pub(crate) fn create_screen_capture_manager_state() -> KmsScreenCaptureManagerState {
    KmsScreenCaptureManagerState::new()
}
