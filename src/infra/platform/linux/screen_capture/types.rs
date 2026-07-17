use std::{io, os::fd::OwnedFd};

use drm::{
    buffer::{DrmFourcc, DrmModifier},
    control::{GetPlanarFramebufferError, framebuffer, plane},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KmsPlaneType {
    Primary,
    Overlay,
    Cursor,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum KmsPlaneCaptureError {
    #[error("failed to query the plane state")]
    QueryPlane(#[source] io::Error),
    #[error("failed to query the plane properties")]
    QueryProperties(#[source] io::Error),
    #[error("failed to query the plane framebuffer")]
    QueryFramebuffer(#[source] GetPlanarFramebufferError),
    #[error("failed to export framebuffer plane {plane_index} as DMA-BUF")]
    ExportBuffer {
        plane_index: usize,
        #[source]
        source: io::Error,
    },
    #[error("active plane is missing required property {property}")]
    MissingProperty { property: &'static str },
    #[error("active plane has invalid {property} value {value}")]
    InvalidProperty { property: &'static str, value: u64 },
    #[error("framebuffer changed from {expected:?} to {actual:?} during capture")]
    SnapshotChanged {
        expected: framebuffer::Handle,
        actual: Option<framebuffer::Handle>,
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
    pub plane_type: KmsPlaneType,
    #[source]
    pub error: KmsPlaneCaptureError,
}
