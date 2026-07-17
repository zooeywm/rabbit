use std::{
    fs::{File, OpenOptions},
    io::ErrorKind,
    os::fd::{AsFd, BorrowedFd},
    path::{Path, PathBuf},
};

use drm::{
    ClientCapability, Device as _,
    control::{Device as _, connector, crtc},
    node::{DrmNode, NodeType},
};
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

        let device = Self {
            file,
            path: path.into(),
        };

        device
            .set_client_capability(ClientCapability::UniversalPlanes, true)
            .with_context(|| {
                format!(
                    "Failed to enable universal DRM planes on {}",
                    device.path().display()
                )
            })?;
        device
            .set_client_capability(ClientCapability::Atomic, true)
            .with_context(|| {
                format!(
                    "Failed to enable atomic DRM properties on {}",
                    device.path().display()
                )
            })?;

        match device.set_client_capability(ClientCapability::CursorPlaneHotspot, true) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::InvalidInput | ErrorKind::Unsupported
                ) => {}
            Err(error) => {
                return Ok(Err(error).with_context(|| {
                    format!(
                        "Failed to enable DRM cursor hotspot support on {}",
                        device.path().display()
                    )
                })?);
            }
        }

        Ok(device)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn find_active_output(
        &self,
        screen_name: &str,
    ) -> eros::Result<Option<(connector::Handle, crtc::Handle, u32)>> {
        let resources = self.resource_handles().with_context(|| {
            format!(
                "Failed to enumerate DRM resources on {}",
                self.path().display()
            )
        })?;

        for connector_handle in resources.connectors() {
            let connector = self
                .get_connector(*connector_handle, false)
                .with_context(|| {
                    format!(
                        "Failed to query DRM connector {connector_handle:?} on {}",
                        self.path().display()
                    )
                })?;

            if connector.to_string() != screen_name {
                continue;
            }

            if connector.state() != connector::State::Connected {
                eros::bail!(
                    "DRM connector {screen_name} on {} is not connected",
                    self.path().display()
                );
            }

            let encoder_handle = connector.current_encoder().with_context(|| {
                format!(
                    "DRM connector {screen_name} on {} has no current encoder",
                    self.path().display()
                )
            })?;
            let encoder = self.get_encoder(encoder_handle).with_context(|| {
                format!(
                    "Failed to query current encoder {encoder_handle:?} for DRM connector \
                     {screen_name} on {}",
                    self.path().display()
                )
            })?;
            let crtc_handle = encoder.crtc().with_context(|| {
                format!(
                    "DRM connector {screen_name} on {} has no active CRTC",
                    self.path().display()
                )
            })?;
            let crtc_index = vblank_crtc_index(resources.crtcs(), crtc_handle)?;

            return Ok(Some((connector.handle(), crtc_handle, crtc_index)));
        }

        Ok(None)
    }
}

fn vblank_crtc_index(crtcs: &[crtc::Handle], target: crtc::Handle) -> eros::Result<u32> {
    let index = crtcs
        .iter()
        .position(|crtc| *crtc == target)
        .with_context(|| format!("DRM CRTC {target:?} is missing from device resources"))?;

    Ok(u32::try_from(index)
        .with_context(|| format!("DRM CRTC {target:?} has an invalid vblank pipe index"))?)
}

impl AsFd for KmsDevice {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl drm::Device for KmsDevice {}

impl drm::control::Device for KmsDevice {}

#[cfg(test)]
mod tests {
    use drm::control::{crtc, from_u32};

    use crate::infra::platform::screen_capture::kms::device::vblank_crtc_index;

    #[test]
    fn resolves_the_vblank_pipe_index_from_resource_order() {
        let first = crtc(10);
        let second = crtc(20);

        assert_eq!(
            vblank_crtc_index(&[first, second], second)
                .expect("second CRTC should have a vblank pipe index"),
            1
        );
        assert!(vblank_crtc_index(&[first], second).is_err());
    }

    fn crtc(raw: u32) -> crtc::Handle {
        from_u32(raw).expect("test CRTC handle should be non-zero")
    }
}
