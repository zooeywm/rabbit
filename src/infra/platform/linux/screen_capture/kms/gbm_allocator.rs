use eros::Context;
use gbm::{BufferObjectFlags, Format};

use crate::{
    infra::platform::{
        dma_buf::DmaBufFrame,
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

        Ok(self.context.allocate_dma_buf(size, format, usage)?)
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
