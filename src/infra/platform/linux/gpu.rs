use std::{
    fs::OpenOptions,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
};

use eros::Context;
use gbm::Device;

use crate::infra::platform::screen_capture::EglContext;

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

    pub(crate) fn device(&self) -> &Device<OwnedFd> {
        &self.device
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::infra::platform::gpu::{GpuContext, GpuDevice};

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
}
