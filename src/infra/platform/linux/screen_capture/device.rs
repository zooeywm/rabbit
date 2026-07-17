use std::{
    fs::{File, OpenOptions},
    os::fd::{AsFd, BorrowedFd},
    path::{Path, PathBuf},
};

use drm::node::{DrmNode, NodeType};
use eros::Context;

#[derive(Debug)]
pub(crate) struct KmsDevice {
    file: File,
    path: PathBuf,
}

impl KmsDevice {
    pub(crate) fn open(path: &Path) -> eros::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to open DRM device {}", path.display()))?;
        let node = DrmNode::from_file(&file)
            .with_context(|| format!("Failed to inspect DRM device {}", path.display()))?;

        if node.ty() != NodeType::Primary {
            eros::bail!("DRM device {} is not a primary node", path.display());
        }

        Ok(Self {
            file,
            path: path.into(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl AsFd for KmsDevice {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl drm::Device for KmsDevice {}

impl drm::control::Device for KmsDevice {}
