use std::{
    fs::OpenOptions,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
};

use eros::Context;
use gbm::{BufferObjectFlags, Device, Format};

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
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gbm::{BufferObjectFlags, Format};

    use crate::infra::platform::gpu::{GpuContext, GpuDevice};
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

        assert!(
            error
                .to_string()
                .contains("/dev/dri/rabbit-missing-render-node")
        );
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

        let frame = context
            .allocate_dma_buf(size, Format::Nv12, BufferObjectFlags::RENDERING)
            .expect("NV12 DMA-BUF output should allocate");

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
}
