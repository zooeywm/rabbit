mod device;
mod types;

#[derive(Debug, kudi::DepInj)]
#[target(KmsScreenCaptureManager)]
pub(crate) struct KmsScreenCaptureManagerState;

impl KmsScreenCaptureManagerState {
    pub(crate) fn new() -> Self {
        Self
    }
}

/// Creates the screen-capture manager state selected for Linux.
pub(crate) fn create_screen_capture_manager_state() -> KmsScreenCaptureManagerState {
    KmsScreenCaptureManagerState::new()
}
