//! DMA-BUF video frame adaptation into GStreamer buffers.
//!
//! Converts GBM pipeline frames into GStreamer memory that owns the DMA-BUF
//! lease for the lifetime of the buffer.

use std::rc::Rc;

use drm::buffer::{DrmFourcc, DrmModifier};
use eros::Context as _;
use gstreamer_allocators::prelude::DmaBufAllocatorExtManual as _;

use crate::infra::platform::{
    dma_buf::{DmaBufFrame, DmaBufLease},
    frame_pipeline::GbmFramePipelineFrame,
    video_probe::VideoFrameProbe,
};
use crate::kernel::geometry::FrameRate;

struct DmaBufLeaseOwner {
    _lease: DmaBufLease,
    storage: [u8; 0],
}

impl AsMut<[u8]> for DmaBufLeaseOwner {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }
}

#[derive(Debug)]
pub(crate) struct GStreamerVideoFrame {
    pub(super) buffer: gstreamer::Buffer,
    pub(super) input_caps: gstreamer::Caps,
    pub(super) input_signature: DmaBufInputSignature,
    pub(super) probe: Option<VideoFrameProbe>,
    pub(super) va_context: Option<gstreamer::Context>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DmaBufInputSignature {
    pub(crate) size: crate::kernel::geometry::PixelSize,
    pub(super) format: DrmFourcc,
    pub(super) modifier: DrmModifier,
    pub(super) frame_rate: FrameRate,
}

impl TryFrom<&gstreamer::CapsRef> for DmaBufInputSignature {
    type Error = eros::ErrorUnion;

    fn try_from(caps: &gstreamer::CapsRef) -> Result<Self, Self::Error> {
        let structure = caps
            .structure(0)
            .with_context(|| "GStreamer DMA-BUF input caps are empty")?;
        let width = structure
            .get::<i32>("width")
            .with_context(|| "GStreamer DMA-BUF input caps do not contain a fixed width")?;
        let height = structure
            .get::<i32>("height")
            .with_context(|| "GStreamer DMA-BUF input caps do not contain a fixed height")?;
        let drm_format = structure
            .get::<&str>("drm-format")
            .with_context(|| "GStreamer DMA-BUF input caps do not contain a DRM format")?;
        let (fourcc, modifier) = gstreamer_video::dma_drm_fourcc_from_str(drm_format)
            .with_context(|| "Failed to parse GStreamer DMA-BUF input DRM format")?;
        let frame_rate = structure
            .get::<gstreamer::Fraction>("framerate")
            .with_context(|| "GStreamer DMA-BUF input caps do not contain a fixed frame rate")?;
        let numerator = u32::try_from(frame_rate.numer())
            .with_context(|| "GStreamer DMA-BUF input frame-rate numerator is not positive")?;
        let denominator = u32::try_from(frame_rate.denom())
            .with_context(|| "GStreamer DMA-BUF input frame-rate denominator is not positive")?;

        Ok(Self {
            size: crate::kernel::geometry::PixelSize {
                width: u32::try_from(width)
                    .with_context(|| "GStreamer DMA-BUF input width is negative")?,
                height: u32::try_from(height)
                    .with_context(|| "GStreamer DMA-BUF input height is negative")?,
            },
            format: DrmFourcc::try_from(fourcc)
                .with_context(|| "GStreamer DMA-BUF input fourcc is unknown")?,
            modifier: DrmModifier::from(modifier),
            frame_rate: FrameRate::new(numerator, denominator)
                .with_context(|| "GStreamer DMA-BUF input frame rate is invalid")?,
        })
    }
}

impl TryFrom<Rc<GbmFramePipelineFrame>> for GStreamerVideoFrame {
    type Error = eros::ErrorUnion;

    fn try_from(source: Rc<GbmFramePipelineFrame>) -> Result<Self, Self::Error> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let frame_rate = source.frame_rate;
        Self::from_pipeline_frame(source, frame_rate, None)
    }
}

impl GStreamerVideoFrame {
    pub(super) fn from_pipeline_frame(
        source: Rc<GbmFramePipelineFrame>,
        frame_rate: FrameRate,
        expected: Option<(&gstreamer::CapsRef, DmaBufInputSignature)>,
    ) -> eros::Result<Self> {
        let frame = &source.buffer;

        if frame.readiness_fence.is_some() {
            eros::bail!("GStreamer input frame still has an unresolved readiness fence");
        }
        let expected_planes = match frame.format {
            DrmFourcc::Nv12 => 2,
            DrmFourcc::Xrgb8888 => 1,
            format => eros::bail!(
                "GStreamer input frame must use NV12 or XRGB8888, got {:?}",
                format
            ),
        };
        if frame.planes.len() != expected_planes {
            eros::bail!(
                "GStreamer {:?} input frame must contain {} planes, got {}",
                frame.format,
                expected_planes,
                frame.planes.len()
            );
        }
        if frame.objects.is_empty() {
            eros::bail!("GStreamer NV12 input frame does not contain DMA-BUF objects");
        }

        let modifier = frame.planes[0].modifier;
        if frame.planes.iter().any(|plane| plane.modifier != modifier) {
            eros::bail!(
                "GStreamer {:?} input frame planes use different DRM modifiers",
                frame.format
            );
        }
        let input_signature = DmaBufInputSignature {
            size: frame.size,
            format: frame.format,
            modifier,
            frame_rate,
        };
        if let Some((_, expected_signature)) = expected
            && input_signature != expected_signature
        {
            eros::bail!(
                "GStreamer encoder input changed from {:?} to {:?}",
                expected_signature,
                input_signature
            );
        }

        const MAX_DMA_BUF_OBJECTS: usize = 2;
        if frame.objects.len() > MAX_DMA_BUF_OBJECTS {
            eros::bail!(
                "GStreamer {:?} input frame contains too many DMA-BUF objects: {}",
                frame.format,
                frame.objects.len()
            );
        }

        let mut object_offsets = [0_usize; MAX_DMA_BUF_OBJECTS];
        let mut buffer_size = 0_usize;
        for (index, object) in frame.objects.iter().enumerate() {
            object_offsets[index] = buffer_size;
            buffer_size = buffer_size
                .checked_add(object.size)
                .with_context(|| "GStreamer DMA-BUF object sizes exceed usize")?;
        }

        let mut offsets = [0_usize; 2];
        let mut strides = [0_i32; 2];
        for (plane_index, plane) in frame.planes.iter().enumerate() {
            let Some(object) = frame.objects.get(plane.object_index) else {
                eros::bail!(
                    "GStreamer NV12 plane {} references missing DMA-BUF object {}",
                    plane_index,
                    plane.object_index
                );
            };
            let plane_offset = usize::try_from(plane.offset)
                .with_context(|| "GStreamer NV12 plane offset exceeds usize")?;
            if plane_offset >= object.size {
                eros::bail!(
                    "GStreamer NV12 plane {} offset {} exceeds DMA-BUF object {} size {}",
                    plane_index,
                    plane.offset,
                    plane.object_index,
                    object.size
                );
            }
            offsets[plane_index] = object_offsets[plane.object_index]
                .checked_add(plane_offset)
                .with_context(|| "GStreamer NV12 plane offset exceeds usize")?;
            strides[plane_index] = i32::try_from(plane.stride)
                .with_context(|| "GStreamer NV12 plane stride exceeds i32")?;
        }

        let input_caps = match expected {
            Some((caps, _)) => caps.to_owned(),
            None => dmabuf_caps(frame, modifier, Some(frame_rate))?,
        };
        let va_context = frame
            .va_backing
            .as_ref()
            .map(|backing| backing.context.clone());
        let mut buffer = match &frame.va_backing {
            Some(backing) => backing
                .buffer
                .copy_region(gstreamer::BufferCopyFlags::MEMORY, ..)
                .with_context(|| "Failed to reference the VA DMA-BUF input memory")?,
            None => {
                let allocator = gstreamer_allocators::DmaBufAllocator::new();
                let mut buffer = gstreamer::Buffer::new();
                let Some(buffer_mut) = buffer.get_mut() else {
                    eros::bail!("New GStreamer DMA-BUF input buffer is unexpectedly shared");
                };
                for (object_index, object) in frame.objects.iter().enumerate() {
                    let fd = object.fd.try_clone().with_context(|| {
                        format!(
                            "Failed to duplicate DMA-BUF object {} for GStreamer",
                            object_index
                        )
                    })?;
                    let memory =
                        unsafe { allocator.alloc_dmabuf(fd, object.size) }.with_context(|| {
                            format!(
                                "Failed to wrap DMA-BUF object {} as GStreamer memory",
                                object_index
                            )
                        })?;
                    buffer_mut.append_memory(memory);
                }
                buffer
            }
        };
        let Some(buffer_mut) = buffer.get_mut() else {
            eros::bail!("New GStreamer DMA-BUF input buffer is unexpectedly shared");
        };
        gstreamer_video::VideoMeta::add_full(
            buffer_mut,
            gstreamer_video::VideoFrameFlags::empty(),
            gstreamer_video::VideoFormat::DmaDrm,
            frame.size.width,
            frame.size.height,
            &offsets[..frame.planes.len()],
            &strides[..frame.planes.len()],
        )
        .with_context(|| "Failed to attach NV12 DMA-BUF layout to GStreamer input frame")?;
        if let Some(lease) = &frame.lease {
            let owner = gstreamer::Buffer::from_mut_slice(DmaBufLeaseOwner {
                _lease: lease.clone(),
                storage: [],
            });
            gstreamer::meta::ParentBufferMeta::add(buffer_mut, &owner);
        }

        validate_dmabuf_buffer(&buffer)?;

        let probe = source.probe.clone();
        if let Some(probe) = &probe {
            let Some(buffer) = buffer.get_mut() else {
                eros::bail!("New GStreamer DMA-BUF input buffer is unexpectedly shared");
            };
            buffer.set_pts(gstreamer::ClockTime::from_nseconds(probe.pts_ns()));
        }

        Ok(Self {
            buffer,
            input_caps,
            input_signature,
            probe,
            va_context,
        })
    }

    pub(crate) fn input_caps(&self) -> &gstreamer::CapsRef {
        &self.input_caps
    }
}

pub(crate) fn validate_dmabuf_buffer(buffer: &gstreamer::BufferRef) -> eros::Result<()> {
    if buffer.n_memory() == 0 {
        eros::bail!("GStreamer video frame does not contain DMA-BUF memory");
    }

    for (index, memory) in buffer.iter_memories().enumerate() {
        if !memory.is_memory_type::<gstreamer_allocators::DmaBufMemory>() {
            eros::bail!("GStreamer video frame memory {} is not DMA-BUF", index);
        }
    }

    let Some(video) = buffer.meta::<gstreamer_video::VideoMeta>() else {
        eros::bail!("GStreamer DMA-BUF video frame is missing VideoMeta");
    };

    if video.format() != gstreamer_video::VideoFormat::DmaDrm {
        eros::bail!(
            "GStreamer DMA-BUF video frame has non-DRM format {}",
            video.format()
        );
    }

    Ok(())
}

pub(crate) fn dmabuf_caps(
    frame: &DmaBufFrame,
    modifier: DrmModifier,
    frame_rate: Option<FrameRate>,
) -> eros::Result<gstreamer::Caps> {
    let width = i32::try_from(frame.size.width)
        .with_context(|| "GStreamer NV12 frame width exceeds i32")?;
    let height = i32::try_from(frame.size.height)
        .with_context(|| "GStreamer NV12 frame height exceeds i32")?;
    let drm_format = if modifier == DrmModifier::Invalid {
        match frame.format {
            DrmFourcc::Nv12 => String::from("NV12"),
            DrmFourcc::Xrgb8888 => String::from("XR24"),
            format => eros::bail!("Unsupported modifierless DMA-BUF format: {:?}", format),
        }
    } else {
        gstreamer_video::dma_drm_fourcc_to_string(frame.format as u32, modifier.into()).to_string()
    };
    let colorimetry = match frame.format {
        DrmFourcc::Nv12 => "bt709",
        DrmFourcc::Xrgb8888 => "sRGB",
        format => eros::bail!("Unsupported DMA-BUF colorimetry for format: {:?}", format),
    };

    let frame_rate = match frame_rate {
        Some(frame_rate) => gstreamer::Fraction::new(
            i32::try_from(frame_rate.numerator())
                .with_context(|| "GStreamer frame-rate numerator exceeds i32")?,
            i32::try_from(frame_rate.denominator())
                .with_context(|| "GStreamer frame-rate denominator exceeds i32")?,
        ),
        None => gstreamer::Fraction::new(0, 1),
    };

    Ok(gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", drm_format)
        .field("width", width)
        .field("height", height)
        .field("framerate", frame_rate)
        .field("interlace-mode", "progressive")
        .field("colorimetry", colorimetry)
        .build())
}
