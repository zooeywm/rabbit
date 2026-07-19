use std::{fs::File, io, os::fd::OwnedFd};

use drm::buffer::{DrmFourcc, DrmModifier};

use crate::kernel::geometry::PixelSize;

#[derive(Debug)]
pub(crate) struct DmaBufObject {
    pub(crate) fd: OwnedFd,
    pub(crate) size: usize,
}

impl TryFrom<OwnedFd> for DmaBufObject {
    type Error = io::Error;

    fn try_from(fd: OwnedFd) -> Result<Self, Self::Error> {
        let file = File::from(fd);
        let size = usize::try_from(file.metadata()?.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "DMA-BUF object length exceeds usize",
            )
        })?;

        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DMA-BUF object has zero length",
            ));
        }

        Ok(Self {
            fd: OwnedFd::from(file),
            size,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DmaBufPlane {
    pub(crate) object_index: usize,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
    pub(crate) modifier: DrmModifier,
}

#[derive(Debug)]
pub(crate) struct DmaBufFrame {
    pub(crate) size: PixelSize,
    pub(crate) format: DrmFourcc,
    pub(crate) objects: Vec<DmaBufObject>,
    pub(crate) planes: Vec<DmaBufPlane>,
    pub(crate) readiness_fence: Option<OwnedFd>,
}
