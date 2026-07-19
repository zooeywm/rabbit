use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::infra::platform::gpu::GpuDevice;

    #[test]
    fn render_node_path_is_the_gpu_identity() {
        let first = GpuDevice::from(PathBuf::from("/dev/dri/renderD128"));
        let same = GpuDevice::from(PathBuf::from("/dev/dri/renderD128"));
        let different = GpuDevice::from(PathBuf::from("/dev/dri/renderD129"));

        assert_eq!(first, same);
        assert_ne!(first, different);
        assert_eq!(first.render_node_path(), Path::new("/dev/dri/renderD128"));
    }
}
