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
    let composition_modifiers = match crate::infra::platform::video_encoder::va_vpp_input_modifiers(
        drm::buffer::DrmFourcc::Xrgb8888,
    ) {
        Ok(modifiers) => modifiers,
        Err(error) => {
            tracing::debug!(
                target: "rabbit::screen_capture::kms",
                error = ?error,
                "VAAPI-compatible KMS composition modifiers are unavailable"
            );
            Vec::new()
        }
    };
    KmsScreenCaptureManagerState::new(enable_probing, worker_reaper, composition_modifiers)
}
