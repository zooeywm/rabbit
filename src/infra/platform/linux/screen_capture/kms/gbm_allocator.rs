use eros::Context;
use gbm::{BufferObjectFlags, Format};

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
        gpu::{GpuContext, GpuDevice},
        screen_capture::kms::{
            composition::KmsCompositionTransform,
            device::KmsDevice,
            types::{KmsFramebufferSnapshot, KmsPlaneIssue},
        },
    },
    kernel::{geometry::PixelSize, screen_capture::CapturedFrame},
};

#[derive(Debug)]
pub(crate) struct GbmFrameAllocator {
    context: GpuContext,
}

impl GbmFrameAllocator {
    pub(crate) fn new(device: &KmsDevice) -> eros::Result<Self> {
        let gpu = GpuDevice::from(device.render_node_path()?);
        let context = GpuContext::new(&gpu)?;

        Ok(Self { context })
    }

    pub(crate) fn allocate_composition_target(&self, size: PixelSize) -> eros::Result<DmaBufFrame> {
        let format = Format::Xrgb8888;
        let usage = BufferObjectFlags::RENDERING;

        if !self.context.device().is_format_supported(format, usage) {
            eros::bail!("GBM does not support {:?} composition targets", format);
        }

        let buffer = self
            .context
            .device()
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
                eros::bail!("GBM returned too many DMA-BUF planes: {}", plane_count);
            }

            let gbm_plane_index = plane_index as i32;
            let fd = buffer.fd_for_plane(gbm_plane_index).with_context(|| {
                format!("Failed to export GBM composition target plane {plane_index} as DMA-BUF")
            })?;
            let object_index = objects.len();

            objects.push(DmaBufObject::try_from(fd).with_context(|| {
                format!("Failed to determine GBM composition target plane {plane_index} length")
            })?);
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

    pub(crate) fn compose(
        &self,
        snapshot: KmsFramebufferSnapshot,
    ) -> eros::Result<CapturedFrame<DmaBufFrame, KmsPlaneIssue>> {
        let KmsFramebufferSnapshot {
            output_size,
            planes,
            mut issues,
        } = snapshot;
        let mut frame = self.allocate_composition_target(output_size)?;
        let image = self.context.egl().import_composition_target(&frame)?;
        let target = self
            .context
            .egl()
            .create_composition_target(&image)
            .with_context(|| "Failed to create the OpenGL KMS composition target")?;

        self.context
            .egl()
            .clear_composition_target(&target)
            .with_context(|| "Failed to initialize the KMS composition target")?;

        for plane in planes {
            let image = match self.context.egl().import_plane(&plane) {
                Ok(image) => image,
                Err(error) => {
                    issues.push(KmsPlaneIssue {
                        plane_id: plane.id,
                        plane_type: Some(plane.plane_type),
                        error,
                    });
                    continue;
                }
            };
            let texture = self
                .context
                .egl()
                .create_external_texture(&image)
                .with_context(|| {
                    format!("Failed to bind KMS plane {:?} for composition", plane.id)
                })?;
            let transform =
                KmsCompositionTransform::new(output_size, plane.buffer.size, plane.placement);

            self.context
                .egl()
                .compose_plane(&target, &texture, &transform, plane.blend)
                .with_context(|| format!("Failed to compose KMS plane {:?}", plane.id))?;
        }

        frame.readiness_fence = Some(
            self.context
                .egl()
                .finish_composition()
                .with_context(|| "Failed to export KMS composition readiness")?,
        );

        Ok(CapturedFrame {
            buffer: frame,
            issues,
        })
    }
}
