use std::os::fd::{AsFd, OwnedFd};

use eros::Context;
use gbm::{BufferObjectFlags, Device, Format};

use crate::{
    infra::platform::screen_capture::kms::{
        device::KmsDevice,
        egl_context::EglContext,
        types::{DmaBufFrame, DmaBufObject, DmaBufPlane},
    },
    kernel::geometry::PixelSize,
};

#[derive(Debug)]
pub(crate) struct GbmFrameAllocator {
    egl: EglContext,
    device: Device<OwnedFd>,
}

impl GbmFrameAllocator {
    pub(crate) fn new(device: &KmsDevice) -> eros::Result<Self> {
        let device_fd = device.as_fd().try_clone_to_owned().with_context(|| {
            format!(
                "Failed to duplicate DRM device {} for GBM",
                device.path().display()
            )
        })?;
        let device = Device::new(device_fd).with_context(|| {
            format!(
                "Failed to create GBM device for {}",
                device.path().display()
            )
        })?;
        let egl = EglContext::new(&device)?;

        Ok(Self { egl, device })
    }

    pub(crate) fn allocate_composition_target(
        &self,
        size: PixelSize,
    ) -> eros::Result<DmaBufFrame> {
        let format = Format::Xrgb8888;
        let usage = BufferObjectFlags::RENDERING;

        if !self.device.is_format_supported(format, usage) {
            eros::bail!("GBM does not support {format:?} composition targets");
        }

        let buffer = self
            .device
            .create_buffer_object::<()>(size.width, size.height, format, usage)
            .with_context(|| {
                format!(
                    "Failed to allocate {format:?} GBM composition target {}x{}",
                    size.width, size.height
                )
            })?;
        let plane_count = buffer.plane_count();

        if plane_count == 0 {
            eros::bail!("GBM allocated a composition target without DMA-BUF planes");
        }

        let modifier = buffer.modifier();
        let mut objects = Vec::with_capacity(plane_count as usize);
        let mut planes = Vec::with_capacity(plane_count as usize);

        for plane_index in 0..plane_count {
            if plane_index > i32::MAX as u32 {
                eros::bail!("GBM returned too many DMA-BUF planes: {plane_count}");
            }

            let gbm_plane_index = plane_index as i32;
            let fd = buffer.fd_for_plane(gbm_plane_index).with_context(|| {
                format!("Failed to export GBM composition target plane {plane_index} as DMA-BUF")
            })?;
            let object_index = objects.len();

            objects.push(DmaBufObject { fd });
            planes.push(DmaBufPlane {
                object_index,
                offset: buffer.offset(gbm_plane_index),
                stride: buffer.stride_for_plane(gbm_plane_index),
                modifier,
            });
        }

        Ok(DmaBufFrame {
            size,
            format,
            objects,
            planes,
            readiness_fence: None,
        })
    }
}
