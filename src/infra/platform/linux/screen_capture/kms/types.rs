use std::io;

use drm::{
    buffer::{DrmFourcc, DrmModifier},
    control::{GetPlanarFramebufferError, PlaneType, crtc, framebuffer, plane},
};

use crate::{infra::platform::dma_buf::DmaBufFrame, kernel::geometry::PixelSize};

#[derive(Debug, thiserror::Error)]
pub(crate) enum KmsPlaneCaptureError {
    #[error("failed to query the plane state")]
    QueryPlane(#[source] io::Error),
    #[error("failed to query the plane properties")]
    QueryProperties(#[source] io::Error),
    #[error("failed to query the plane framebuffer")]
    QueryFramebuffer(#[source] GetPlanarFramebufferError),
    #[error("failed to export framebuffer object {object_index} as DMA-BUF")]
    ExportBuffer {
        object_index: usize,
        #[source]
        source: io::Error,
    },
    #[error("failed to determine DMA-BUF object {object_index} length")]
    QueryBufferSize {
        object_index: usize,
        #[source]
        source: io::Error,
    },
    #[error("failed to duplicate cached framebuffer DMA-BUF objects: {reason}")]
    CloneCachedBuffer { reason: String },
    #[error("failed to close temporary GEM handle for framebuffer object {object_index}")]
    CloseBuffer {
        object_index: usize,
        #[source]
        source: io::Error,
    },
    #[error("GETFB2 did not return a GEM handle for framebuffer plane {plane_index}")]
    MissingBufferHandle { plane_index: usize },
    #[error("DMA-BUF does not contain any planes")]
    MissingDmaBufPlanes,
    #[error("DMA-BUF contains {count} planes, but EGL supports at most {maximum}")]
    TooManyDmaBufPlanes { count: usize, maximum: usize },
    #[error("DMA-BUF plane {plane_index} references missing object {object_index}")]
    MissingDmaBufObject {
        plane_index: usize,
        object_index: usize,
    },
    #[error("failed to import format {format:?} with modifier {modifier:?} as an EGLImage")]
    ImportDmaBuf {
        format: DrmFourcc,
        modifier: DrmModifier,
        #[source]
        source: khronos_egl::Error,
    },
    #[error("failed to bind a cached framebuffer EGLImage as an OpenGL texture: {reason}")]
    BindCachedImage { reason: String },
    #[error("plane is missing required property {property}")]
    MissingProperty { property: &'static str },
    #[error("plane has invalid {property} value {value}")]
    InvalidProperty { property: &'static str, value: u64 },
    #[error(
        "plane state changed during capture: CRTC {expected_crtc:?} -> {actual_crtc:?}, \
         framebuffer {expected_framebuffer:?} -> {actual_framebuffer:?}"
    )]
    SnapshotChanged {
        expected_crtc: crtc::Handle,
        actual_crtc: Option<crtc::Handle>,
        expected_framebuffer: framebuffer::Handle,
        actual_framebuffer: Option<framebuffer::Handle>,
    },
    #[error("GPU composition does not support format {format:?} with modifier {modifier:?}")]
    UnsupportedFormat {
        format: DrmFourcc,
        modifier: DrmModifier,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("KMS {plane_type:?} plane {plane_id:?}: {error}")]
pub(crate) struct KmsPlaneIssue {
    pub plane_id: plane::Handle,
    pub plane_type: Option<PlaneType>,
    #[source]
    pub error: KmsPlaneCaptureError,
}

#[derive(Debug)]
pub(crate) struct KmsActivePlane {
    pub id: plane::Handle,
    pub plane_type: PlaneType,
    pub framebuffer: framebuffer::Handle,
    pub placement: KmsPlanePlacement,
    pub blend: KmsPlaneBlend,
    pub color: KmsPlaneColor,
    pub cursor_hotspot: Option<KmsCursorHotspot>,
}

#[derive(Debug)]
pub(crate) struct KmsPlaneSnapshot {
    pub output_size: PixelSize,
    pub planes: Vec<KmsActivePlane>,
    pub issues: Vec<KmsPlaneIssue>,
}

#[derive(Debug)]
pub(crate) struct KmsFramebufferPlane {
    pub id: plane::Handle,
    pub plane_type: PlaneType,
    pub buffer: DmaBufFrame,
    pub placement: KmsPlanePlacement,
    pub blend: KmsPlaneBlend,
    pub color: KmsPlaneColor,
    pub cursor_hotspot: Option<KmsCursorHotspot>,
    pub cache_key: KmsFramebufferCacheKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KmsFramebufferCacheKey(pub(crate) u64);

#[derive(Debug)]
pub(crate) struct KmsFramebufferSnapshot {
    pub output_size: PixelSize,
    pub planes: Vec<KmsFramebufferPlane>,
    pub issues: Vec<KmsPlaneIssue>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct KmsPlanePlacement {
    pub zpos: u64,
    pub source: KmsSourceRect,
    pub destination: KmsDestinationRect,
    pub transform: KmsPlaneTransform,
}

/// Source coordinates in DRM's unsigned 16.16 fixed-point representation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KmsSourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct KmsDestinationRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KmsPlaneTransform {
    pub rotation: KmsRotation,
    pub reflect_x: bool,
    pub reflect_y: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KmsRotation {
    #[default]
    Rotate0,
    Rotate90,
    Rotate180,
    Rotate270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KmsPlaneBlend {
    pub alpha: u16,
    pub pixel_mode: KmsPixelBlendMode,
}

impl Default for KmsPlaneBlend {
    fn default() -> Self {
        Self {
            alpha: u16::MAX,
            pixel_mode: KmsPixelBlendMode::PreMultiplied,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KmsPixelBlendMode {
    None,
    #[default]
    PreMultiplied,
    Coverage,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KmsPlaneColor {
    pub encoding: KmsColorEncoding,
    pub range: KmsColorRange,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KmsColorEncoding {
    #[default]
    Bt601,
    Bt709,
    Bt2020,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KmsColorRange {
    #[default]
    Limited,
    Full,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KmsCursorHotspot {
    pub x: u32,
    pub y: u32,
}
