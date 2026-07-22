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
    render_node_path: PathBuf,
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

        Ok(Self {
            egl,
            device,
            render_node_path: render_node_path.to_owned(),
        })
    }

    pub(crate) fn egl(&self) -> &EglContext {
        &self.egl
    }

    pub(crate) fn render_node_path(&self) -> &Path {
        &self.render_node_path
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
            va_backing: None,
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
            va_backing: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        os::unix::fs::MetadataExt as _,
        path::{Path, PathBuf},
        time::Duration,
    };

    use compio::runtime::fd::PollFd;
    use eros::Context as _;
    use gbm::Format;

    use crate::infra::platform::{
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
        gpu::{GpuContext, GpuDevice, Nv12OutputStrategy},
        video_encoder::{VaDmaBufAllocator, hardware_h264_encoder_for, va_vpp_input_modifier},
    };
    use crate::kernel::geometry::PixelSize;

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

        let allocator = VaDmaBufAllocator::new(&render_node, size)
            .expect("VAAPI should initialize an encoder-compatible NV12 allocator");
        let mut frame = allocator
            .allocate()
            .expect("VAAPI should allocate an encoder-compatible NV12 surface");
        let other_frames = [
            allocator
                .allocate()
                .expect("VAAPI should allocate a second NV12 surface"),
            allocator
                .allocate()
                .expect("VAAPI should allocate a third NV12 surface"),
        ];
        let surface_inode = |frame: &DmaBufFrame| {
            File::from(
                frame.objects[0]
                    .fd
                    .try_clone()
                    .expect("VA surface DMA-BUF fd should duplicate"),
            )
            .metadata()
            .expect("VA surface DMA-BUF should expose metadata")
            .ino()
        };
        let surface_inodes = [
            surface_inode(&frame),
            surface_inode(&other_frames[0]),
            surface_inode(&other_frames[1]),
        ];
        assert_ne!(surface_inodes[0], surface_inodes[1]);
        assert_ne!(surface_inodes[0], surface_inodes[2]);
        assert_ne!(surface_inodes[1], surface_inodes[2]);
        assert_eq!(frame.format, Format::Nv12);
        assert_eq!(frame.planes.len(), 2);
        let target_image = context
            .egl()
            .import_nv12_target(&frame)
            .expect("EGL should import the VA NV12 surface");
        let target = context
            .egl()
            .create_nv12_target(&target_image)
            .expect("OpenGL should bind the VA NV12 surface");
        context
            .egl()
            .convert_to_nv12(&source_texture, &target)
            .expect("OpenGL should write the VA NV12 surface");
        frame.readiness_fence = Some(
            context
                .egl()
                .finish_frame_pipeline()
                .expect("OpenGL should export VA surface readiness"),
        );
        let fence = frame
            .readiness_fence
            .take()
            .expect("VA surface should retain its OpenGL readiness fence");
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");
        runtime.block_on(async {
            compio::time::timeout(Duration::from_secs(2), async {
                PollFd::new(fence)
                    .expect("VA surface readiness fence should register")
                    .read_ready()
                    .await
                    .with_context(|| "Failed to wait for VA surface readiness fence")
            })
            .await
            .expect("VA surface readiness fence should signal within two seconds")
            .expect("VA surface readiness fence should signal successfully");
        });
        let encoder = hardware_h264_encoder_for(&frame)
            .expect("A hardware H.264 encoder should accept the VA NV12 surface");

        println!(
            "VA surface probe: encoder={encoder}, modifier={:?}, objects={}, planes={}",
            frame.planes[0].modifier,
            frame.objects.len(),
            frame.planes.len()
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
            va_backing: None,
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
