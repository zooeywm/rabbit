//! Multi-plane KMS composition into a single DMA-BUF target.

use eros::Context as _;
use gbm::Format;

use crate::infra::platform::{
    dma_buf::{DmaBufFrame, DmaBufPool},
    gpu::GpuContext,
    screen_capture::{KmsCompositionTransform, KmsFramebufferPlane, KmsPlaneIssue},
};
use crate::kernel::screen_manager::ScreenId;

pub(crate) struct GpuComposition {
    pub(super) size: Option<crate::kernel::geometry::PixelSize>,
    pub(super) pool: DmaBufPool,
    pub(super) selected: bool,
    pub(super) pool_exhaustion_warned: bool,
    pub(super) fallback_requested: bool,
}

const COMPOSITION_POOL_CAPACITY: usize = 3;
/// Recording holds composition targets until VA encode releases them.
const RECORD_COMPOSITION_POOL_CAPACITY: usize = 48;

impl GpuComposition {
    pub(super) fn new() -> Self {
        Self {
            size: None,
            pool: DmaBufPool::new(COMPOSITION_POOL_CAPACITY),
            selected: false,
            pool_exhaustion_warned: false,
            fallback_requested: false,
        }
    }

    pub(super) fn ensure_pool_capacity(&mut self, capacity: usize) {
        if self.pool.capacity() >= capacity {
            return;
        }
        let size = self.size;
        self.pool = DmaBufPool::new(capacity);
        self.size = size;
        self.pool_exhaustion_warned = false;
    }
}

pub(super) fn compose_plane_set(
    context: &GpuContext,
    screen_id: ScreenId,
    output_size: crate::kernel::geometry::PixelSize,
    planes: &[KmsFramebufferPlane],
    issues: &mut Vec<KmsPlaneIssue>,
    composition: &mut GpuComposition,
    block_on_pool_exhaustion: bool,
) -> eros::Result<Option<DmaBufFrame>> {
    let desired_capacity = if block_on_pool_exhaustion {
        RECORD_COMPOSITION_POOL_CAPACITY
    } else {
        COMPOSITION_POOL_CAPACITY
    };
    if composition.size != Some(output_size) {
        composition.size = Some(output_size);
        composition.pool = DmaBufPool::new(desired_capacity);
        composition.pool_exhaustion_warned = false;
    } else if block_on_pool_exhaustion {
        composition.ensure_pool_capacity(desired_capacity);
    }

    let wait_fence = |fence| {
        context
            .egl()
            .wait_on_native_fence(fence)
            .with_context(|| "Failed to enqueue fused KMS composition-target reuse fence")
    };
    let allocate = || {
        context.allocate_dma_buf(
            output_size,
            Format::Xrgb8888,
            gbm::BufferObjectFlags::RENDERING,
        )
    };

    let frame = if block_on_pool_exhaustion {
        composition.pool.acquire_blocking(allocate, wait_fence)?
    } else {
        let Some(frame) = composition.pool.acquire(allocate, wait_fence)? else {
            if !composition.pool_exhaustion_warned {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    screen_id = screen_id.0,
                    capacity = composition.pool.capacity(),
                    "Fused KMS composition pool exhausted; dropping source frames until a target is released"
                );
                composition.pool_exhaustion_warned = true;
            }
            return Ok(None);
        };
        frame
    };

    let result = compose_plane_set_into(context, screen_id, output_size, planes, issues, &frame);
    if let Err(error) = result {
        frame.invalidate_lease();
        return Err(error);
    }

    if !composition.selected {
        tracing::info!(
            target: "rabbit::frame_pipeline",
            screen_id = screen_id.0,
            width = output_size.width,
            height = output_size.height,
            plane_count = planes.len(),
            strategy = "gpu_fused_kms_composition",
            "Selected KMS composition strategy"
        );
        composition.selected = true;
    }

    Ok(Some(frame))
}

pub(super) fn compose_plane_set_into(
    context: &GpuContext,
    screen_id: ScreenId,
    output_size: crate::kernel::geometry::PixelSize,
    planes: &[KmsFramebufferPlane],
    issues: &mut Vec<KmsPlaneIssue>,
    frame: &DmaBufFrame,
) -> eros::Result<()> {
    let image = context
        .egl()
        .import_composition_target(frame)
        .with_context(|| "Failed to import the fused KMS composition target")?;
    let target = context
        .egl()
        .create_composition_target(&image)
        .with_context(|| "Failed to bind the fused KMS composition target")?;
    context
        .egl()
        .clear_composition_target(&target)
        .with_context(|| "Failed to clear the fused KMS composition target")?;
    context.egl().retain_cached_plane_images(planes);

    for plane in planes {
        let texture = match context.egl().create_cached_plane_texture(plane) {
            Ok(texture) => texture,
            Err(error) => {
                tracing::warn!(
                    target: "rabbit::screen_capture::kms",
                    screen_id = screen_id.0,
                    plane_id = ?plane.id,
                    plane_type = ?plane.plane_type,
                    error = ?error,
                    "Skipped a KMS plane during fused GPU composition"
                );
                issues.push(KmsPlaneIssue {
                    plane_id: plane.id,
                    plane_type: Some(plane.plane_type),
                    error,
                });
                continue;
            }
        };
        let transform = KmsCompositionTransform::new(
            output_size,
            plane.buffer.size,
            plane.placement,
            plane.cursor_hotspot,
        );
        context
            .egl()
            .compose_plane(&target, &texture, &transform, plane.blend)
            .with_context(|| {
                format!(
                    "Failed to compose KMS plane {:?} on the GPU worker",
                    plane.id
                )
            })?;
    }

    Ok(())
}
