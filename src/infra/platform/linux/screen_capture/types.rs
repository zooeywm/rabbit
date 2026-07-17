use std::{io, os::fd::OwnedFd};

use drm::{
    buffer::{DrmFourcc, DrmModifier},
    control::{GetPlanarFramebufferError, PlaneType, crtc, framebuffer, plane},
};

use crate::kernel::geometry::PixelSize;

#[derive(Debug)]
pub(crate) struct DmaBufObject {
    pub fd: OwnedFd,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DmaBufPlane {
    pub object_index: usize,
    pub offset: u32,
    pub stride: u32,
    pub modifier: DrmModifier,
}

#[derive(Debug)]
pub(crate) struct DmaBufFrame {
    pub size: PixelSize,
    pub format: DrmFourcc,
    pub objects: Vec<DmaBufObject>,
    pub planes: Vec<DmaBufPlane>,
    pub readiness_fence: Option<OwnedFd>,
}

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
    #[error("failed to close temporary GEM handle for framebuffer object {object_index}")]
    CloseBuffer {
        object_index: usize,
        #[source]
        source: io::Error,
    },
    #[error("GETFB2 did not return a GEM handle for framebuffer plane {plane_index}")]
    MissingBufferHandle { plane_index: usize },
    #[error("active plane is missing required property {property}")]
    MissingProperty { property: &'static str },
    #[error("active plane has invalid {property} value {value}")]
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
    #[error("plane contains protected content")]
    ProtectedContent,
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
}

#[derive(Debug)]
pub(crate) struct KmsPlaneSnapshot {
    pub planes: Vec<KmsActivePlane>,
    pub issues: Vec<KmsPlaneIssue>,
}

#[derive(Debug)]
pub(crate) struct KmsFramebufferPlane {
    pub id: plane::Handle,
    pub plane_type: PlaneType,
    pub buffer: DmaBufFrame,
    pub placement: KmsPlanePlacement,
}

#[derive(Debug)]
pub(crate) struct KmsFramebufferSnapshot {
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
