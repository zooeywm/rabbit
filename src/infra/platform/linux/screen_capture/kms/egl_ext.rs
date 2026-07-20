use khronos_egl as egl;

pub(crate) use crate::infra::platform::egl_dma_buf::{
    DMA_BUF_PLANE_FD_EXT, DMA_BUF_PLANE_MODIFIER_HI_EXT, DMA_BUF_PLANE_MODIFIER_LO_EXT,
    DMA_BUF_PLANE_OFFSET_EXT, DMA_BUF_PLANE_PITCH_EXT, ITU_REC601_EXT, ITU_REC709_EXT,
    ITU_REC2020_EXT, LINUX_DMA_BUF_EXT, LINUX_DRM_FOURCC_EXT, SAMPLE_RANGE_HINT_EXT,
    YUV_COLOR_SPACE_HINT_EXT, YUV_FULL_RANGE_EXT, YUV_NARROW_RANGE_EXT,
};

// EGL_ANDROID_native_fence_sync
pub(crate) const SYNC_NATIVE_FENCE_ANDROID: egl::Enum = 0x3144;
pub(crate) const SYNC_NATIVE_FENCE_FD_ANDROID: egl::Attrib = 0x3145;
pub(crate) const NO_NATIVE_FENCE_FD_ANDROID: egl::Int = -1;
pub(crate) type DupNativeFenceFdAndroid =
    unsafe extern "system" fn(egl::EGLDisplay, egl::EGLSync) -> egl::Int;

// EGL_MESA_image_dma_buf_export
#[cfg(test)]
pub(crate) type ExportDmaBufImageQueryMesa = unsafe extern "system" fn(
    egl::EGLDisplay,
    egl::EGLImage,
    *mut egl::Int,
    *mut egl::Int,
    *mut u64,
) -> egl::Boolean;
#[cfg(test)]
pub(crate) type ExportDmaBufImageMesa = unsafe extern "system" fn(
    egl::EGLDisplay,
    egl::EGLImage,
    *mut egl::Int,
    *mut egl::Int,
    *mut egl::Int,
) -> egl::Boolean;

// EGL_KHR_gl_texture_2D_image
#[cfg(test)]
pub(crate) const GL_TEXTURE_2D_KHR: egl::Enum = 0x30B1;
#[cfg(test)]
pub(crate) const GL_TEXTURE_LEVEL_KHR: egl::Attrib = 0x30BC;

// EGL_KHR_platform_gbm and EGL_MESA_platform_gbm use the same platform token.
pub(crate) const PLATFORM_GBM_KHR: egl::Enum = 0x31D7;
