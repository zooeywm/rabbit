use std::{fs, path::PathBuf};

use drm::control::{connector, crtc};
use eros::Context;

use crate::infra::platform::screen_capture::device::KmsDevice;

const DRM_CLASS_PATH: &str = "/sys/class/drm";
const DRM_DEVICE_PATH: &str = "/dev/dri";

#[derive(Debug)]
pub(crate) struct KmsOutput {
    pub device: KmsDevice,
    pub connector: connector::Handle,
    pub crtc: crtc::Handle,
}

impl KmsOutput {
    pub(crate) fn open(screen_name: &str) -> eros::Result<Self> {
        let mut device_paths = device_paths(screen_name)?;
        device_paths.sort_unstable();

        if device_paths.is_empty() {
            eros::bail!("No DRM connector matches screen {screen_name}");
        }

        let mut output = None;

        for device_path in device_paths {
            let device = KmsDevice::open(&device_path)?;
            let Some((connector, crtc)) = device.find_active_output(screen_name)? else {
                continue;
            };

            if output.is_some() {
                eros::bail!("More than one active DRM connector matches screen {screen_name}");
            }

            output = Some(Self {
                device,
                connector,
                crtc,
            });
        }

        Ok(output.with_context(|| format!("No active DRM connector matches screen {screen_name}"))?)
    }
}

fn device_paths(screen_name: &str) -> eros::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(DRM_CLASS_PATH)
        .with_context(|| format!("Failed to enumerate {DRM_CLASS_PATH}"))?;
    let mut paths = Vec::new();

    for entry in entries {
        let entry = entry.with_context(|| format!("Failed to read an entry in {DRM_CLASS_PATH}"))?;
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().with_context(|| {
            format!("DRM class entry name {entry_name:?} is not valid UTF-8")
        })?;
        let Some((card_name, connector_name)) = entry_name.split_once('-') else {
            continue;
        };
        let Some(card_index) = card_name.strip_prefix("card") else {
            continue;
        };

        if connector_name == screen_name
            && !card_index.is_empty()
            && card_index.bytes().all(|byte| byte.is_ascii_digit())
        {
            paths.push(PathBuf::from(DRM_DEVICE_PATH).join(card_name));
        }
    }

    Ok(paths)
}
