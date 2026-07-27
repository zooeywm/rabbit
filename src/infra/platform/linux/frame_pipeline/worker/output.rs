//! Encoder-facing output strategy selection for processed frames.

use eros::Context as _;
use gbm::{Format, Modifier};

use crate::infra::platform::{
    gpu::{GpuContext, Nv12OutputStrategy},
    video_encoder::{VaDmaBufAllocator, hardware_h264_encoder_for, va_vpp_input_modifiers},
};

use super::GpuPipeline;

#[derive(Debug, Clone, Copy)]
pub(crate) enum FrameOutputStrategy {
    PassthroughXrgb(Modifier),
    VaDirectNv12,
    DirectNv12(Nv12OutputStrategy),
    VaapiXrgb(Modifier),
}

impl FrameOutputStrategy {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::PassthroughXrgb(_) => "kms_xrgb_passthrough",
            Self::VaDirectNv12 => "va_direct_nv12",
            Self::DirectNv12(strategy) => strategy.name(),
            Self::VaapiXrgb(_) => "vaapi_xrgb",
        }
    }
}

pub(super) fn select_output(
    context: &GpuContext,
    pipeline: &mut GpuPipeline,
) -> eros::Result<(
    crate::infra::platform::dma_buf::DmaBufFrame,
    FrameOutputStrategy,
)> {
    if let Some(output) = select_direct_nv12_output(context, pipeline) {
        return Ok(output);
    }

    let size = pipeline.parameters.frame_size;

    for modifier in va_vpp_input_modifiers(Format::Xrgb8888)
        .with_context(|| "Failed to find VAAPI-compatible XRGB DMA-BUF modifiers")?
    {
        let strategy = FrameOutputStrategy::VaapiXrgb(modifier);
        let buffer = match allocate_output(context, size, strategy, None) {
            Ok(buffer) => buffer,
            Err(error) => {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    modifier = ?modifier,
                    error = ?error,
                    "Failed to allocate VAAPI XRGB output candidate"
                );
                continue;
            }
        };
        let renderable = context
            .egl()
            .import_composition_target(&buffer)
            .and_then(|image| context.egl().create_composition_target(&image));
        if let Err(error) = renderable {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                modifier = ?modifier,
                error = ?error,
                "EGL rejected VAAPI XRGB output candidate"
            );
            continue;
        }
        if let Err(error) = hardware_h264_encoder_for(&buffer) {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                modifier = ?modifier,
                error = ?error,
                "Hardware H.264 pipeline rejected VAAPI XRGB output candidate"
            );
            continue;
        }

        return Ok((buffer, strategy));
    }

    eros::bail!("No VAAPI XRGB DMA-BUF output candidate is usable")
}

pub(super) fn select_direct_nv12_output(
    context: &GpuContext,
    pipeline: &mut GpuPipeline,
) -> Option<(
    crate::infra::platform::dma_buf::DmaBufFrame,
    FrameOutputStrategy,
)> {
    let size = pipeline.parameters.frame_size;
    let va_allocator = match VaDmaBufAllocator::new(context.render_node_path(), size) {
        Ok(allocator) => Some(allocator),
        Err(error) => {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                strategy = FrameOutputStrategy::VaDirectNv12.name(),
                error = ?error,
                "Failed to initialize VA DirectNV12 output candidate"
            );
            None
        }
    };
    if let Some(va_allocator) = va_allocator {
        let buffer = match va_allocator.allocate() {
            Ok(buffer) => Some(buffer),
            Err(error) => {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    strategy = FrameOutputStrategy::VaDirectNv12.name(),
                    error = ?error,
                    "Failed to allocate VA DirectNV12 output candidate"
                );
                None
            }
        };
        if let Some(buffer) = buffer {
            let renderable = context
                .egl()
                .import_nv12_target(&buffer)
                .and_then(|image| context.egl().create_nv12_target(&image));
            if let Err(error) = renderable {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    strategy = FrameOutputStrategy::VaDirectNv12.name(),
                    error = ?error,
                    "EGL rejected VA DirectNV12 output candidate"
                );
            } else if let Err(error) = hardware_h264_encoder_for(&buffer) {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    strategy = FrameOutputStrategy::VaDirectNv12.name(),
                    error = ?error,
                    "Hardware H.264 encoder rejected VA DirectNV12 output candidate"
                );
            } else {
                pipeline.va_nv12_allocator = Some(va_allocator);
                return Some((buffer, FrameOutputStrategy::VaDirectNv12));
            }
        }
    }

    for strategy in Nv12OutputStrategy::ALL {
        if !context.supports_nv12_output(strategy) {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                strategy = strategy.name(),
                "Direct NV12 output candidate is unsupported"
            );
            continue;
        }

        let buffer = match context.allocate_nv12_output(size, strategy) {
            Ok(buffer) => buffer,
            Err(error) => {
                tracing::warn!(
                    target: "rabbit::frame_pipeline",
                    strategy = strategy.name(),
                    error = ?error,
                    "Failed to allocate direct NV12 output candidate"
                );
                continue;
            }
        };
        let renderable = context
            .egl()
            .import_nv12_target(&buffer)
            .and_then(|image| context.egl().create_nv12_target(&image));
        if let Err(error) = renderable {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                strategy = strategy.name(),
                error = ?error,
                "EGL rejected direct NV12 output candidate"
            );
            continue;
        }
        if let Err(error) = hardware_h264_encoder_for(&buffer) {
            tracing::warn!(
                target: "rabbit::frame_pipeline",
                strategy = strategy.name(),
                error = ?error,
                "Hardware H.264 encoder rejected direct NV12 output candidate"
            );
            continue;
        }

        return Some((buffer, FrameOutputStrategy::DirectNv12(strategy)));
    }

    None
}

pub(super) fn allocate_output(
    context: &GpuContext,
    size: crate::kernel::geometry::PixelSize,
    strategy: FrameOutputStrategy,
    va_nv12_allocator: Option<&VaDmaBufAllocator>,
) -> eros::Result<crate::infra::platform::dma_buf::DmaBufFrame> {
    match strategy {
        FrameOutputStrategy::PassthroughXrgb(_) => {
            eros::bail!("XRGB pass-through does not allocate a frame-pipeline output")
        }
        FrameOutputStrategy::VaDirectNv12 => va_nv12_allocator
            .with_context(|| "VA DirectNV12 strategy has no allocator")?
            .allocate(),
        FrameOutputStrategy::DirectNv12(strategy) => context.allocate_nv12_output(size, strategy),
        FrameOutputStrategy::VaapiXrgb(modifier) => context.allocate_dma_buf_with_modifier(
            size,
            Format::Xrgb8888,
            modifier,
            gbm::BufferObjectFlags::RENDERING,
        ),
    }
}

pub(super) fn validate_nv12_size(size: crate::kernel::geometry::PixelSize) -> eros::Result<()> {
    if size.width == 0 || size.height == 0 {
        eros::bail!(
            "NV12 frame size must be non-zero, got {}x{}",
            size.width,
            size.height
        );
    }

    if !size.width.is_multiple_of(2) || !size.height.is_multiple_of(2) {
        eros::bail!(
            "NV12 frame size must use even dimensions, got {}x{}",
            size.width,
            size.height
        );
    }

    Ok(())
}
