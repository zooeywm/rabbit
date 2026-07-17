mod composition;
mod device;
mod egl_context;
mod egl_ext;
mod framebuffer;
mod gbm_allocator;
mod gl_context;
mod output;
mod types;

#[derive(Debug, kudi::DepInj)]
#[target(KmsScreenCaptureManager)]
pub(crate) struct KmsScreenCaptureManagerState;

impl KmsScreenCaptureManagerState {
    pub(crate) fn new() -> Self {
        Self
    }
}
