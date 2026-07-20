use eros::Context;
use gbm::{BufferObjectFlags, Format, Modifier};

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufPool},
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
    preferred_modifiers: Vec<Modifier>,
    composition_strategy: Option<CompositionTargetStrategy>,
    pool_size: Option<PixelSize>,
    pool: DmaBufPool,
}

const COMPOSITION_POOL_CAPACITY: usize = 3;

#[derive(Debug, Clone, Copy)]
enum CompositionTargetStrategy {
    PreferredModifier(Modifier),
    Generic,
}

impl GbmFrameAllocator {
    pub(crate) fn new(
        device: &KmsDevice,
        preferred_modifiers: Vec<Modifier>,
    ) -> eros::Result<Self> {
        let gpu = GpuDevice::from(device.render_node_path()?);
        let context = GpuContext::new(&gpu)?;

        Ok(Self {
            context,
            preferred_modifiers,
            composition_strategy: None,
            pool_size: None,
            pool: DmaBufPool::new(COMPOSITION_POOL_CAPACITY),
        })
    }

    fn allocate_composition_target(&mut self, size: PixelSize) -> eros::Result<DmaBufFrame> {
        let format = Format::Xrgb8888;
        let usage = BufferObjectFlags::RENDERING;

        if let Some(strategy) = self.composition_strategy {
            return match strategy {
                CompositionTargetStrategy::PreferredModifier(modifier) => self
                    .context
                    .allocate_dma_buf_with_modifier(size, format, modifier, usage),
                CompositionTargetStrategy::Generic => {
                    self.context.allocate_dma_buf(size, format, usage)
                }
            };
        }

        for modifier in self.preferred_modifiers.iter().copied() {
            let frame = match self
                .context
                .allocate_dma_buf_with_modifier(size, format, modifier, usage)
            {
                Ok(frame) => frame,
                Err(error) => {
                    tracing::debug!(
                        target: "rabbit::screen_capture::kms",
                        ?modifier,
                        error = ?error,
                        "KMS compositor rejected preferred XRGB modifier"
                    );
                    continue;
                }
            };
            let renderable = self
                .context
                .egl()
                .import_composition_target(&frame)
                .and_then(|image| self.context.egl().create_composition_target(&image));
            if let Err(error) = renderable {
                tracing::debug!(
                    target: "rabbit::screen_capture::kms",
                    ?modifier,
                    error = ?error,
                    "EGL rejected preferred KMS composition modifier"
                );
                continue;
            }

            self.composition_strategy =
                Some(CompositionTargetStrategy::PreferredModifier(modifier));
            tracing::info!(
                target: "rabbit::screen_capture::kms",
                ?modifier,
                "Selected encoder-compatible KMS composition modifier"
            );
            return Ok(frame);
        }

        let frame = self.context.allocate_dma_buf(size, format, usage)?;
        let modifier = frame
            .planes
            .first()
            .map(|plane| plane.modifier)
            .unwrap_or(drm::buffer::DrmModifier::Invalid);
        self.composition_strategy = Some(CompositionTargetStrategy::Generic);
        tracing::info!(
            target: "rabbit::screen_capture::kms",
            ?modifier,
            "Selected generic KMS composition target"
        );
        Ok(frame)
    }

    pub(crate) fn compose(
        &mut self,
        snapshot: KmsFramebufferSnapshot,
    ) -> eros::Result<Option<CapturedFrame<DmaBufFrame, KmsPlaneIssue>>> {
        let KmsFramebufferSnapshot {
            output_size,
            planes,
            mut issues,
        } = snapshot;
        if self.pool_size != Some(output_size) {
            self.pool = DmaBufPool::new(COMPOSITION_POOL_CAPACITY);
            self.pool_size = Some(output_size);
        }
        let mut first = if self.composition_strategy.is_none() {
            Some(self.allocate_composition_target(output_size)?)
        } else {
            None
        };
        let context = &self.context;
        let composition_strategy = self
            .composition_strategy
            .with_context(|| "KMS composition-target strategy was not selected")?;
        let frame = self.pool.acquire(
            || match first.take() {
                Some(frame) => Ok(frame),
                None => allocate_composition_target(context, output_size, composition_strategy),
            },
            |fence| {
                context
                    .egl()
                    .wait_on_native_fence(fence)
                    .with_context(|| "Failed to enqueue KMS composition-target reuse fence")
            },
        );
        let Some(mut frame) = frame? else {
            return Ok(None);
        };
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

        self.context.egl().retain_cached_plane_images(&planes);

        for plane in planes {
            let texture = match self.context.egl().create_cached_plane_texture(&plane) {
                Ok(texture) => texture,
                Err(error) => {
                    issues.push(KmsPlaneIssue {
                        plane_id: plane.id,
                        plane_type: Some(plane.plane_type),
                        error,
                    });
                    continue;
                }
            };
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

        Ok(Some(CapturedFrame {
            buffer: frame,
            issues,
        }))
    }
}

fn allocate_composition_target(
    context: &GpuContext,
    size: PixelSize,
    strategy: CompositionTargetStrategy,
) -> eros::Result<DmaBufFrame> {
    let usage = BufferObjectFlags::RENDERING;

    match strategy {
        CompositionTargetStrategy::PreferredModifier(modifier) => {
            context.allocate_dma_buf_with_modifier(size, Format::Xrgb8888, modifier, usage)
        }
        CompositionTargetStrategy::Generic => {
            context.allocate_dma_buf(size, Format::Xrgb8888, usage)
        }
    }
}
