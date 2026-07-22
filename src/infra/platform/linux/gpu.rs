use std::{
    fs::OpenOptions,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
};

use eros::Context;
use gbm::{BufferObject, BufferObjectFlags, Device, Format, Modifier};

use crate::{
    infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
        screen_capture::EglContext,
    },
    kernel::geometry::PixelSize,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GpuDevice {
    render_node_path: PathBuf,
}

impl GpuDevice {
    pub(crate) fn render_node_path(&self) -> &Path {
        &self.render_node_path
    }
}

impl From<PathBuf> for GpuDevice {
    fn from(render_node_path: PathBuf) -> Self {
        Self { render_node_path }
    }
}

#[derive(Debug)]
pub(crate) struct GpuContext {
    egl: EglContext,
    device: Device<OwnedFd>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nv12OutputStrategy {
    GbmNv12,
    GbmSeparatePlanes,
}

impl Nv12OutputStrategy {
    pub(crate) const ALL: [Self; 2] = [Self::GbmNv12, Self::GbmSeparatePlanes];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::GbmNv12 => "gbm_nv12",
            Self::GbmSeparatePlanes => "gbm_separate_planes",
        }
    }
}

impl GpuContext {
    pub(crate) fn new(gpu: &GpuDevice) -> eros::Result<Self> {
        let render_node_path = gpu.render_node_path();
        let render_node = OpenOptions::new()
            .read(true)
            .write(true)
            .open(render_node_path)
            .with_context(|| {
                format!(
                    "Failed to open DRM render node {}",
                    render_node_path.display()
                )
            })?;
        let device = Device::new(OwnedFd::from(render_node)).with_context(|| {
            format!(
                "Failed to create GBM device from {}",
                render_node_path.display()
            )
        })?;
        let egl = EglContext::new(&device).with_context(|| {
            format!(
                "Failed to initialize EGL/OpenGL on {}",
                render_node_path.display()
            )
        })?;

        Ok(Self { egl, device })
    }

    pub(crate) fn egl(&self) -> &EglContext {
        &self.egl
    }

    pub(crate) fn allocate_dma_buf(
        &self,
        size: PixelSize,
        format: Format,
        usage: BufferObjectFlags,
    ) -> eros::Result<DmaBufFrame> {
        if !self.device.is_format_supported(format, usage) {
            eros::bail!(
                "GBM does not support {:?} buffers with usage {:?}",
                format,
                usage
            );
        }

        let buffer = self
            .device
            .create_buffer_object::<()>(size.width, size.height, format, usage)
            .with_context(|| {
                format!(
                    "Failed to allocate {format:?} GBM buffer {}x{}",
                    size.width, size.height
                )
            })?;
        self.export_buffer(size, format, buffer)
    }

    pub(crate) fn allocate_dma_buf_with_modifier(
        &self,
        size: PixelSize,
        format: Format,
        modifier: Modifier,
        usage: BufferObjectFlags,
    ) -> eros::Result<DmaBufFrame> {
        let buffer = self
            .device
            .create_buffer_object_with_modifiers2::<()>(
                size.width,
                size.height,
                format,
                std::iter::once(modifier),
                usage,
            )
            .with_context(|| {
                format!(
                    "Failed to allocate {format:?} GBM buffer {}x{} with modifier {modifier:?}",
                    size.width, size.height
                )
            })?;

        self.export_buffer(size, format, buffer)
    }

    fn export_buffer(
        &self,
        size: PixelSize,
        format: Format,
        buffer: BufferObject<()>,
    ) -> eros::Result<DmaBufFrame> {
        let plane_count = buffer.plane_count();

        if plane_count == 0 {
            eros::bail!("GBM allocated a {:?} buffer without DMA-BUF planes", format);
        }

        let modifier = buffer.modifier();
        let mut objects = Vec::with_capacity(plane_count as usize);
        let mut planes = Vec::with_capacity(plane_count as usize);

        for plane_index in 0..plane_count {
            if plane_index > i32::MAX as u32 {
                eros::bail!(
                    "GBM returned too many {:?} DMA-BUF planes: {}",
                    format,
                    plane_count
                );
            }

            let gbm_plane_index = plane_index as i32;
            let fd = buffer.fd_for_plane(gbm_plane_index).with_context(|| {
                format!("Failed to export {format:?} GBM plane {plane_index} as DMA-BUF")
            })?;
            let object_index = objects.len();

            objects.push(DmaBufObject::try_from(fd).with_context(|| {
                format!("Failed to determine {format:?} GBM plane {plane_index} length")
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
            lease: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn select_nv12_output(
        &self,
        size: PixelSize,
    ) -> eros::Result<(DmaBufFrame, Nv12OutputStrategy)> {
        for strategy in Nv12OutputStrategy::ALL {
            if !self.supports_nv12_output(strategy) {
                tracing::debug!(
                    target: "rabbit::frame_pipeline",
                    strategy = strategy.name(),
                    "NV12 output strategy is unsupported"
                );
                continue;
            }

            match self.allocate_nv12_output(size, strategy) {
                Ok(frame) => return Ok((frame, strategy)),
                Err(error) => tracing::debug!(
                    target: "rabbit::frame_pipeline",
                    strategy = strategy.name(),
                    error = ?error,
                    "NV12 output strategy allocation failed"
                ),
            }
        }

        eros::bail!(
            "GBM supports neither a renderable NV12 buffer nor renderable R8 and GR88 planes"
        )
    }

    pub(crate) fn allocate_nv12_output(
        &self,
        size: PixelSize,
        strategy: Nv12OutputStrategy,
    ) -> eros::Result<DmaBufFrame> {
        let usage = BufferObjectFlags::RENDERING;

        match strategy {
            Nv12OutputStrategy::GbmNv12 => self.allocate_dma_buf(size, Format::Nv12, usage),
            Nv12OutputStrategy::GbmSeparatePlanes => {
                self.allocate_separate_nv12_dma_buf(size, usage)
            }
        }
    }

    pub(crate) fn supports_nv12_output(&self, strategy: Nv12OutputStrategy) -> bool {
        let usage = BufferObjectFlags::RENDERING;

        match strategy {
            Nv12OutputStrategy::GbmNv12 => self.device.is_format_supported(Format::Nv12, usage),
            Nv12OutputStrategy::GbmSeparatePlanes => {
                self.device.is_format_supported(Format::R8, usage)
                    && self.device.is_format_supported(Format::Gr88, usage)
            }
        }
    }

    fn allocate_separate_nv12_dma_buf(
        &self,
        size: PixelSize,
        usage: BufferObjectFlags,
    ) -> eros::Result<DmaBufFrame> {
        let chroma_size = PixelSize {
            width: size.width / 2,
            height: size.height / 2,
        };
        let mut luma = self.allocate_dma_buf(size, Format::R8, usage)?;
        let chroma = self.allocate_dma_buf(chroma_size, Format::Gr88, usage)?;

        if luma.planes.len() != 1 || chroma.planes.len() != 1 {
            eros::bail!(
                "Separate NV12 output requires one R8 plane and one GR88 plane, got {} and {}",
                luma.planes.len(),
                chroma.planes.len()
            );
        }
        if luma.planes[0].modifier != chroma.planes[0].modifier {
            eros::bail!(
                "Separate NV12 output planes use different modifiers: {:?} and {:?}",
                luma.planes[0].modifier,
                chroma.planes[0].modifier
            );
        }

        let chroma_object_offset = luma.objects.len();
        luma.objects.extend(chroma.objects);
        let mut chroma_plane = chroma.planes[0];
        chroma_plane.object_index += chroma_object_offset;

        Ok(DmaBufFrame {
            size,
            format: Format::Nv12,
            objects: luma.objects,
            planes: vec![luma.planes[0], chroma_plane],
            readiness_fence: None,
            lease: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::{CString, c_void},
        os::{
            fd::{BorrowedFd, OwnedFd},
            unix::ffi::OsStrExt as _,
        },
        path::{Path, PathBuf},
        ptr::NonNull,
    };

    use eros::Context as _;
    use gbm::Format;
    use gstreamer::glib::translate::{FromGlibPtrFull as _, ToGlibPtr as _, from_glib};

    use crate::infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
        gpu::{GpuContext, GpuDevice, Nv12OutputStrategy},
        video_encoder::{hardware_h264_encoder_for, va_vpp_input_modifier},
    };
    use crate::kernel::geometry::PixelSize;

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
    }

    struct VaDisplay(NonNull<c_void>);

    impl Drop for VaDisplay {
        fn drop(&mut self) {
            unsafe {
                gstreamer::glib::gobject_ffi::g_object_unref(self.0.as_ptr().cast());
            }
        }
    }

    struct VaDmaBufSurface {
        frame: DmaBufFrame,
        _owner: gstreamer::Buffer,
        _allocator: gstreamer::Allocator,
        _display: VaDisplay,
    }

    fn allocate_va_nv12_surface(
        render_node: &Path,
        size: PixelSize,
    ) -> eros::Result<VaDmaBufSurface> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let render_node = CString::new(render_node.as_os_str().as_bytes())
            .with_context(|| "VA render-node path contains a NUL byte")?;
        let display =
            NonNull::new(unsafe { gst_va_display_drm_new_from_path(render_node.as_ptr()) })
                .with_context(|| "Failed to create a GStreamer VA display")?;
        let display = VaDisplay(display);
        let allocator = NonNull::new(unsafe { gst_va_dmabuf_allocator_new(display.0.as_ptr()) })
            .with_context(|| "Failed to create a GStreamer VA DMA-BUF allocator")?;
        let allocator = unsafe { gstreamer::Allocator::from_glib_full(allocator.as_ptr()) };
        let usage_hint = VA_SURFACE_ATTRIB_USAGE_HINT_ENCODER;
        let modifier = unsafe {
            gst_va_dmabuf_get_modifier_for_format(
                display.0.as_ptr(),
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
        let mut drm_info =
            gstreamer_video::VideoInfoDmaDrm::new(video_info, Format::Nv12 as u32, modifier);
        let configured: bool = unsafe {
            from_glib(gst_va_dmabuf_allocator_set_format(
                allocator.to_glib_none().0,
                (&mut drm_info as *mut gstreamer_video::VideoInfoDmaDrm)
                    .cast::<gstreamer_video::ffi::GstVideoInfoDmaDrm>(),
                usage_hint,
            ))
        };
        if !configured {
            eros::bail!(
                "Failed to configure the GStreamer VA DMA-BUF allocator for modifier {:?} and usage hint 0x{:08X}",
                drm::buffer::DrmModifier::from(modifier),
                usage_hint
            );
        }

        let mut owner = gstreamer::Buffer::new();
        let allocated: bool = unsafe {
            from_glib(gst_va_dmabuf_allocator_setup_buffer(
                allocator.to_glib_none().0,
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

        let modifier = drm::buffer::DrmModifier::from(drm_info.modifier());
        let mut planes = Vec::with_capacity(drm_info.n_planes() as usize);
        for (plane_index, (&offset, &stride)) in
            drm_info.offset().iter().zip(drm_info.stride()).enumerate()
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

        Ok(VaDmaBufSurface {
            frame: DmaBufFrame {
                size,
                format: Format::try_from(drm_info.fourcc())
                    .with_context(|| "VA surface has an unknown DRM fourcc")?,
                objects,
                planes,
                readiness_fence: None,
                lease: None,
            },
            _owner: owner,
            _allocator: allocator,
            _display: display,
        })
    }

    #[test]
    fn render_node_path_is_the_gpu_identity() {
        let first = GpuDevice::from(PathBuf::from("/dev/dri/renderD128"));
        let same = GpuDevice::from(PathBuf::from("/dev/dri/renderD128"));
        let different = GpuDevice::from(PathBuf::from("/dev/dri/renderD129"));

        assert_eq!(first, same);
        assert_ne!(first, different);
        assert_eq!(first.render_node_path(), Path::new("/dev/dri/renderD128"));
    }

    #[test]
    fn context_error_identifies_the_render_node() {
        let gpu = GpuDevice::from(PathBuf::from("/dev/dri/rabbit-missing-render-node"));

        let error = GpuContext::new(&gpu).expect_err("Missing render node should fail");

        assert!(format!("{error:?}").contains("/dev/dri/rabbit-missing-render-node"));
    }

    #[test]
    #[ignore = "run through scripts/test-gpu"]
    fn allocates_nv12_dma_buf_output() {
        let render_node = std::env::var_os("RABBIT_GPU_RENDER_NODE")
            .expect("RABBIT_GPU_RENDER_NODE should name the render node under test");
        let gpu = GpuDevice::from(PathBuf::from(render_node));
        let context = GpuContext::new(&gpu).expect("GPU context should initialize");
        let size = PixelSize {
            width: 1280,
            height: 720,
        };

        let (frame, strategy) = context
            .select_nv12_output(size)
            .expect("NV12 DMA-BUF output should allocate");

        assert!(matches!(
            strategy,
            Nv12OutputStrategy::GbmNv12 | Nv12OutputStrategy::GbmSeparatePlanes
        ));
        assert_eq!(frame.size, size);
        assert_eq!(frame.format, Format::Nv12);
        assert_eq!(frame.planes.len(), 2);
        assert_eq!(frame.objects.len(), frame.planes.len());
        assert!(frame.planes.iter().all(|plane| plane.stride > 0));

        let image = context
            .egl()
            .import_nv12_target(&frame)
            .expect("NV12 output planes should import into EGL");
        let _target = context
            .egl()
            .create_nv12_target(&image)
            .expect("NV12 output planes should bind as OpenGL targets");
    }

    // Focused probe: RABBIT_GPU_RENDER_NODE=/dev/dri/renderD128 cargo test --lib infra::platform::gpu::tests::writes_an_encoder_va_surface_with_opengl -- --ignored --nocapture
    #[test]
    #[ignore = "requires a real DRM render node and VAAPI hardware encoder"]
    fn writes_an_encoder_va_surface_with_opengl() {
        let render_node = std::env::var_os("RABBIT_GPU_RENDER_NODE")
            .expect("RABBIT_GPU_RENDER_NODE should name the render node under test");
        let render_node = PathBuf::from(render_node);
        let gpu = GpuDevice::from(render_node.clone());
        let context = GpuContext::new(&gpu).expect("GPU context should initialize");
        let size = PixelSize {
            width: 2880,
            height: 1800,
        };
        let mut source = context
            .allocate_dma_buf(size, Format::Xrgb8888, gbm::BufferObjectFlags::RENDERING)
            .expect("GBM should allocate the XRGB probe source");
        let source_image = context
            .egl()
            .import_composition_target(&source)
            .expect("EGL should import the XRGB probe source");
        let source_target = context
            .egl()
            .create_composition_target(&source_image)
            .expect("OpenGL should bind the XRGB probe source");
        context
            .egl()
            .clear_composition_target(&source_target)
            .expect("OpenGL should initialize the XRGB probe source");
        let source_fence = context
            .egl()
            .finish_composition()
            .expect("OpenGL should finish the XRGB probe source");
        context
            .egl()
            .wait_on_native_fence(source_fence)
            .expect("OpenGL should wait for the XRGB probe source");
        source.readiness_fence = None;
        let source_image = context
            .egl()
            .import_dma_buf_frame(&source)
            .expect("EGL should import the initialized XRGB probe source");
        let source_texture = context
            .egl()
            .create_dma_buf_texture(&source_image)
            .expect("OpenGL should bind the XRGB probe source texture");

        let mut surface = allocate_va_nv12_surface(&render_node, size)
            .expect("VAAPI should allocate an encoder-compatible NV12 surface");
        assert_eq!(surface.frame.format, Format::Nv12);
        assert_eq!(surface.frame.planes.len(), 2);
        let target_image = context
            .egl()
            .import_nv12_target(&surface.frame)
            .expect("EGL should import the VA NV12 surface");
        let target = context
            .egl()
            .create_nv12_target(&target_image)
            .expect("OpenGL should bind the VA NV12 surface");
        context
            .egl()
            .convert_to_nv12(&source_texture, &target)
            .expect("OpenGL should write the VA NV12 surface");
        surface.frame.readiness_fence = Some(
            context
                .egl()
                .finish_frame_pipeline()
                .expect("OpenGL should export VA surface readiness"),
        );
        let encoder = hardware_h264_encoder_for(&surface.frame)
            .expect("A hardware H.264 encoder should accept the VA NV12 surface");

        println!(
            "VA surface probe: encoder={encoder}, modifier={:?}, objects={}, planes={}",
            surface.frame.planes[0].modifier,
            surface.frame.objects.len(),
            surface.frame.planes.len()
        );
    }

    #[test]
    #[ignore = "run through scripts/test-gpu"]
    fn egl_exported_nv12_planes_match_a_hardware_encoder() {
        let render_node = std::env::var_os("RABBIT_GPU_RENDER_NODE")
            .expect("RABBIT_GPU_RENDER_NODE should name the render node under test");
        let gpu = GpuDevice::from(PathBuf::from(render_node));
        let context = GpuContext::new(&gpu).expect("GPU context should initialize");
        let size = PixelSize {
            width: 1280,
            height: 720,
        };
        let luma = context
            .egl()
            .export_texture_plane(size, Format::R8)
            .expect("EGL should export an R8 texture as DMA-BUF");
        let chroma_size = PixelSize {
            width: size.width / 2,
            height: size.height / 2,
        };
        let chroma = context
            .egl()
            .export_texture_plane(chroma_size, Format::Gr88)
            .expect("EGL should export a GR88 texture as DMA-BUF");

        assert_eq!(luma.format, Format::R8);
        assert_eq!(chroma.format, Format::Gr88);
        assert_eq!(luma.modifier, chroma.modifier);

        let modifier = luma.modifier;
        let luma_object = DmaBufObject::try_from(luma.fd)
            .expect("EGL luma DMA-BUF should have a discoverable size");
        let chroma_object = DmaBufObject::try_from(chroma.fd)
            .expect("EGL chroma DMA-BUF should have a discoverable size");
        let frame = DmaBufFrame {
            size,
            format: Format::Nv12,
            objects: vec![luma_object, chroma_object],
            planes: vec![
                DmaBufPlane {
                    object_index: 0,
                    offset: luma.offset,
                    stride: luma.stride,
                    modifier,
                },
                DmaBufPlane {
                    object_index: 1,
                    offset: chroma.offset,
                    stride: chroma.stride,
                    modifier,
                },
            ],
            readiness_fence: None,
            lease: None,
        };
        println!(
            "EGL export probe: luma={:?}, chroma={:?}, modifier={:?}",
            luma.format, chroma.format, modifier
        );
        let encoder = hardware_h264_encoder_for(&frame)
            .expect("A hardware H.264 encoder should accept the exported DMA-BUF modifier");

        println!("EGL export probe encoder: {encoder}");
    }

    #[test]
    #[ignore = "run through scripts/test-gpu"]
    fn allocates_a_vaapi_importable_egl_composition_target() {
        let render_node = std::env::var_os("RABBIT_GPU_RENDER_NODE")
            .expect("RABBIT_GPU_RENDER_NODE should name the render node under test");
        let gpu = GpuDevice::from(PathBuf::from(render_node));
        let context = GpuContext::new(&gpu).expect("GPU context should initialize");
        let size = PixelSize {
            width: 1280,
            height: 720,
        };
        let modifier = va_vpp_input_modifier(Format::Xrgb8888)
            .expect("VAAPI VPP should advertise an XRGB DMA-BUF modifier");
        let frame = context
            .allocate_dma_buf_with_modifier(
                size,
                Format::Xrgb8888,
                modifier,
                gbm::BufferObjectFlags::RENDERING,
            )
            .expect("GBM should allocate the VAAPI-compatible XRGB composition target");
        let image = context
            .egl()
            .import_composition_target(&frame)
            .expect("EGL should import the VAAPI-compatible XRGB composition target");
        let target = context
            .egl()
            .create_composition_target(&image)
            .expect("OpenGL should bind the VAAPI-compatible XRGB composition target");
        context
            .egl()
            .clear_composition_target(&target)
            .expect("OpenGL should render into the VAAPI-compatible XRGB composition target");
        let _fence = context
            .egl()
            .finish_composition()
            .expect("OpenGL should export the composition readiness fence");

        println!(
            "VAAPI input probe: format={:?}, modifier={:?}, planes={}",
            frame.format,
            modifier,
            frame.planes.len()
        );
    }
}
