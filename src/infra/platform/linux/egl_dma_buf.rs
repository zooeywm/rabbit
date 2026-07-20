use khronos_egl as egl;

pub(crate) const LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
pub(crate) const LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
pub(crate) const YUV_COLOR_SPACE_HINT_EXT: egl::Attrib = 0x327B;
pub(crate) const SAMPLE_RANGE_HINT_EXT: egl::Attrib = 0x327C;
pub(crate) const ITU_REC601_EXT: egl::Attrib = 0x327F;
pub(crate) const ITU_REC709_EXT: egl::Attrib = 0x3280;
pub(crate) const ITU_REC2020_EXT: egl::Attrib = 0x3281;
pub(crate) const YUV_FULL_RANGE_EXT: egl::Attrib = 0x3282;
pub(crate) const YUV_NARROW_RANGE_EXT: egl::Attrib = 0x3283;

pub(crate) const DMA_BUF_PLANE_FD_EXT: [egl::Attrib; 4] = [0x3272, 0x3275, 0x3278, 0x3440];
pub(crate) const DMA_BUF_PLANE_OFFSET_EXT: [egl::Attrib; 4] = [0x3273, 0x3276, 0x3279, 0x3441];
pub(crate) const DMA_BUF_PLANE_PITCH_EXT: [egl::Attrib; 4] = [0x3274, 0x3277, 0x327A, 0x3442];
pub(crate) const DMA_BUF_PLANE_MODIFIER_LO_EXT: [egl::Attrib; 4] = [0x3443, 0x3445, 0x3447, 0x3449];
pub(crate) const DMA_BUF_PLANE_MODIFIER_HI_EXT: [egl::Attrib; 4] = [0x3444, 0x3446, 0x3448, 0x344A];
