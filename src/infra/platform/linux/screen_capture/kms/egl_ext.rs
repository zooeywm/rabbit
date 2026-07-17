use khronos_egl as egl;

// EGL_KHR_platform_gbm and EGL_MESA_platform_gbm use the same platform token.
pub(crate) const PLATFORM_GBM_KHR: egl::Enum = 0x31D7;

// EGL_EXT_image_dma_buf_import
pub(crate) const LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
pub(crate) const LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
const DMA_BUF_PLANE0_FD_EXT: egl::Attrib = 0x3272;
const DMA_BUF_PLANE0_OFFSET_EXT: egl::Attrib = 0x3273;
const DMA_BUF_PLANE0_PITCH_EXT: egl::Attrib = 0x3274;
const DMA_BUF_PLANE1_FD_EXT: egl::Attrib = 0x3275;
const DMA_BUF_PLANE1_OFFSET_EXT: egl::Attrib = 0x3276;
const DMA_BUF_PLANE1_PITCH_EXT: egl::Attrib = 0x3277;
const DMA_BUF_PLANE2_FD_EXT: egl::Attrib = 0x3278;
const DMA_BUF_PLANE2_OFFSET_EXT: egl::Attrib = 0x3279;
const DMA_BUF_PLANE2_PITCH_EXT: egl::Attrib = 0x327A;
pub(crate) const YUV_COLOR_SPACE_HINT_EXT: egl::Attrib = 0x327B;
pub(crate) const SAMPLE_RANGE_HINT_EXT: egl::Attrib = 0x327C;
pub(crate) const ITU_REC601_EXT: egl::Attrib = 0x327F;
pub(crate) const ITU_REC709_EXT: egl::Attrib = 0x3280;
pub(crate) const ITU_REC2020_EXT: egl::Attrib = 0x3281;
pub(crate) const YUV_FULL_RANGE_EXT: egl::Attrib = 0x3282;
pub(crate) const YUV_NARROW_RANGE_EXT: egl::Attrib = 0x3283;
const DMA_BUF_PLANE3_FD_EXT: egl::Attrib = 0x3440;
const DMA_BUF_PLANE3_OFFSET_EXT: egl::Attrib = 0x3441;
const DMA_BUF_PLANE3_PITCH_EXT: egl::Attrib = 0x3442;

// EGL_EXT_image_dma_buf_import_modifiers
const DMA_BUF_PLANE0_MODIFIER_LO_EXT: egl::Attrib = 0x3443;
const DMA_BUF_PLANE0_MODIFIER_HI_EXT: egl::Attrib = 0x3444;
const DMA_BUF_PLANE1_MODIFIER_LO_EXT: egl::Attrib = 0x3445;
const DMA_BUF_PLANE1_MODIFIER_HI_EXT: egl::Attrib = 0x3446;
const DMA_BUF_PLANE2_MODIFIER_LO_EXT: egl::Attrib = 0x3447;
const DMA_BUF_PLANE2_MODIFIER_HI_EXT: egl::Attrib = 0x3448;
const DMA_BUF_PLANE3_MODIFIER_LO_EXT: egl::Attrib = 0x3449;
const DMA_BUF_PLANE3_MODIFIER_HI_EXT: egl::Attrib = 0x344A;

pub(crate) const DMA_BUF_PLANE_FD_EXT: [egl::Attrib; 4] = [
    DMA_BUF_PLANE0_FD_EXT,
    DMA_BUF_PLANE1_FD_EXT,
    DMA_BUF_PLANE2_FD_EXT,
    DMA_BUF_PLANE3_FD_EXT,
];
pub(crate) const DMA_BUF_PLANE_OFFSET_EXT: [egl::Attrib; 4] = [
    DMA_BUF_PLANE0_OFFSET_EXT,
    DMA_BUF_PLANE1_OFFSET_EXT,
    DMA_BUF_PLANE2_OFFSET_EXT,
    DMA_BUF_PLANE3_OFFSET_EXT,
];
pub(crate) const DMA_BUF_PLANE_PITCH_EXT: [egl::Attrib; 4] = [
    DMA_BUF_PLANE0_PITCH_EXT,
    DMA_BUF_PLANE1_PITCH_EXT,
    DMA_BUF_PLANE2_PITCH_EXT,
    DMA_BUF_PLANE3_PITCH_EXT,
];
pub(crate) const DMA_BUF_PLANE_MODIFIER_LO_EXT: [egl::Attrib; 4] = [
    DMA_BUF_PLANE0_MODIFIER_LO_EXT,
    DMA_BUF_PLANE1_MODIFIER_LO_EXT,
    DMA_BUF_PLANE2_MODIFIER_LO_EXT,
    DMA_BUF_PLANE3_MODIFIER_LO_EXT,
];
pub(crate) const DMA_BUF_PLANE_MODIFIER_HI_EXT: [egl::Attrib; 4] = [
    DMA_BUF_PLANE0_MODIFIER_HI_EXT,
    DMA_BUF_PLANE1_MODIFIER_HI_EXT,
    DMA_BUF_PLANE2_MODIFIER_HI_EXT,
    DMA_BUF_PLANE3_MODIFIER_HI_EXT,
];
