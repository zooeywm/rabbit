use std::{
    ffi::{CString, c_void},
    os::{
        fd::{BorrowedFd, OwnedFd},
        unix::ffi::OsStrExt as _,
    },
    path::Path,
    ptr::NonNull,
};

use drm::buffer::{DrmFourcc, DrmModifier};
use eros::Context as _;
use gstreamer::glib::object::ObjectType as _;
use gstreamer::glib::translate::{FromGlibPtrFull as _, ToGlibPtr as _, from_glib};

use crate::{
    infra::platform::dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane, DmaBufVaBacking},
    kernel::geometry::PixelSize,
};

const VA_SURFACE_ATTRIB_USAGE_HINT_ENCODER: u32 = 0x0000_0002;

#[link(name = "gstva-1.0")]
unsafe extern "C" {
    fn gst_va_display_drm_new_from_path(path: *const std::ffi::c_char) -> *mut c_void;
    fn gst_va_dmabuf_allocator_new(display: *mut c_void) -> *mut gstreamer::ffi::GstAllocator;
    fn gst_va_dmabuf_get_modifier_for_format(
        display: *mut c_void,
        format: gstreamer_video::ffi::GstVideoFormat,
        usage_hint: u32,
    ) -> u64;
    fn gst_va_dmabuf_allocator_set_format(
        allocator: *mut gstreamer::ffi::GstAllocator,
        info: *mut gstreamer_video::ffi::GstVideoInfoDmaDrm,
        usage_hint: u32,
    ) -> gstreamer::glib::ffi::gboolean;
    fn gst_va_dmabuf_allocator_setup_buffer(
        allocator: *mut gstreamer::ffi::GstAllocator,
        buffer: *mut gstreamer::ffi::GstBuffer,
    ) -> gstreamer::glib::ffi::gboolean;
    fn gst_context_set_va_display(context: *mut gstreamer::ffi::GstContext, display: *mut c_void);
}

#[derive(Debug)]
pub(crate) struct VaDmaBufAllocator {
    allocator: gstreamer::Allocator,
    info: gstreamer_video::VideoInfoDmaDrm,
    context: gstreamer::Context,
    _display: gstreamer::Object,
}

impl VaDmaBufAllocator {
    pub(crate) fn new(render_node: &Path, size: PixelSize) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let render_node = CString::new(render_node.as_os_str().as_bytes())
            .with_context(|| "VA render-node path contains a NUL byte")?;
        let display =
            NonNull::new(unsafe { gst_va_display_drm_new_from_path(render_node.as_ptr()) })
                .with_context(|| "Failed to create a GStreamer VA display")?;
        let display = unsafe { gstreamer::Object::from_glib_full(display.as_ptr().cast()) };
        let allocator =
            NonNull::new(unsafe { gst_va_dmabuf_allocator_new(display.as_ptr().cast()) })
                .with_context(|| "Failed to create a GStreamer VA DMA-BUF allocator")?;
        let allocator = unsafe { gstreamer::Allocator::from_glib_full(allocator.as_ptr()) };
        let usage_hint = VA_SURFACE_ATTRIB_USAGE_HINT_ENCODER;
        let modifier = unsafe {
            gst_va_dmabuf_get_modifier_for_format(
                display.as_ptr().cast(),
                gstreamer_video::ffi::GST_VIDEO_FORMAT_NV12,
                usage_hint,
            )
        };
        let video_info = gstreamer_video::VideoInfo::builder(
            gstreamer_video::VideoFormat::Nv12,
            size.width,
            size.height,
        )
        .build()
        .with_context(|| "Failed to describe the VA NV12 surface")?;
        let mut info =
            gstreamer_video::VideoInfoDmaDrm::new(video_info, DrmFourcc::Nv12 as u32, modifier);
        let configured: bool = unsafe {
            from_glib(gst_va_dmabuf_allocator_set_format(
                allocator.to_glib_none().0,
                (&mut info as *mut gstreamer_video::VideoInfoDmaDrm)
                    .cast::<gstreamer_video::ffi::GstVideoInfoDmaDrm>(),
                usage_hint,
            ))
        };
        if !configured {
            eros::bail!(
                "Failed to configure the GStreamer VA DMA-BUF allocator for modifier {:?}",
                DrmModifier::from(modifier)
            );
        }

        let context = gstreamer::Context::new("gst.va.display.handle", true);
        unsafe {
            gst_context_set_va_display(context.to_glib_none().0, display.as_ptr().cast());
        }

        Ok(Self {
            allocator,
            info,
            context,
            _display: display,
        })
    }

    pub(crate) fn allocate(&self) -> eros::Result<DmaBufFrame> {
        let mut owner = gstreamer::Buffer::new();
        let allocated: bool = unsafe {
            from_glib(gst_va_dmabuf_allocator_setup_buffer(
                self.allocator.to_glib_none().0,
                owner
                    .get_mut()
                    .with_context(|| "New VA DMA-BUF buffer is unexpectedly shared")?
                    .as_mut_ptr(),
            ))
        };
        if !allocated {
            eros::bail!("Failed to allocate a GStreamer VA DMA-BUF surface");
        }
        if owner.n_memory() == 0 {
            eros::bail!("GStreamer VA DMA-BUF surface has no memory objects");
        }

        let mut objects = Vec::with_capacity(owner.n_memory());
        for (object_index, memory) in owner.iter_memories().enumerate() {
            let dma_buf = memory
                .downcast_memory_ref::<gstreamer_allocators::DmaBufMemory>()
                .with_context(|| format!("VA surface memory {object_index} is not a DMA-BUF"))?;
            let borrowed = unsafe { BorrowedFd::borrow_raw(dma_buf.fd()) };
            let fd: OwnedFd = borrowed.try_clone_to_owned().with_context(|| {
                format!("Failed to duplicate VA surface DMA-BUF object {object_index}")
            })?;
            objects.push(DmaBufObject::try_from(fd).with_context(|| {
                format!("Failed to inspect VA surface DMA-BUF object {object_index}")
            })?);
        }

        let modifier = DrmModifier::from(self.info.modifier());
        let mut planes = Vec::with_capacity(self.info.n_planes() as usize);
        for (plane_index, (&offset, &stride)) in self
            .info
            .offset()
            .iter()
            .zip(self.info.stride())
            .enumerate()
        {
            let (memory_range, skip) = owner
                .find_memory(offset..offset.saturating_add(1))
                .with_context(|| {
                    format!("Failed to locate VA surface plane {plane_index} memory")
                })?;
            if memory_range.len() != 1 {
                eros::bail!(
                    "VA surface plane {} spans {} memory objects",
                    plane_index,
                    memory_range.len()
                );
            }
            let memory = owner.peek_memory(memory_range.start);
            let object_offset = memory
                .offset()
                .checked_add(skip)
                .with_context(|| "VA surface plane offset exceeds usize")?;
            planes.push(DmaBufPlane {
                object_index: memory_range.start,
                offset: u32::try_from(object_offset)
                    .with_context(|| "VA surface plane offset exceeds u32")?,
                stride: u32::try_from(stride)
                    .with_context(|| "VA surface plane stride is negative")?,
                modifier,
            });
        }

        Ok(DmaBufFrame {
            size: PixelSize {
                width: self.info.width(),
                height: self.info.height(),
            },
            format: DrmFourcc::try_from(self.info.fourcc())
                .with_context(|| "VA surface has an unknown DRM fourcc")?,
            objects,
            planes,
            readiness_fence: None,
            lease: None,
            va_backing: Some(DmaBufVaBacking {
                buffer: owner,
                context: self.context.clone(),
            }),
        })
    }
}

// Focused hardware test: RABBIT_GPU_RENDER_NODE=/dev/dri/renderD128 cargo test --lib infra::platform::gpu::tests::writes_an_encoder_va_surface_with_opengl -- --ignored --nocapture
