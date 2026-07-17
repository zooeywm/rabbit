use std::{fs, path::PathBuf};

use drm::control::{Device as _, PlaneType, connector, crtc, plane};
use eros::Context;

use crate::infra::platform::screen_capture::{
    device::KmsDevice,
    types::{
        KmsActivePlane, KmsPlaneCaptureError, KmsPlaneIssue, KmsPlaneSnapshot,
    },
};

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

    pub(crate) fn snapshot_planes(&self) -> eros::Result<KmsPlaneSnapshot> {
        let plane_handles = self.device.plane_handles().with_context(|| {
            format!(
                "Failed to enumerate DRM planes on {}",
                self.device.path().display()
            )
        })?;
        let mut planes = Vec::new();
        let mut issues = Vec::new();

        for plane_id in plane_handles {
            let plane = match self.device.get_plane(plane_id) {
                Ok(plane) => plane,
                Err(source) => {
                    issues.push(KmsPlaneIssue {
                        plane_id,
                        plane_type: None,
                        error: KmsPlaneCaptureError::QueryPlane(source),
                    });
                    continue;
                }
            };

            if plane.crtc() != Some(self.crtc) {
                continue;
            }

            let Some(framebuffer) = plane.framebuffer() else {
                continue;
            };

            let plane_type = match query_plane_type(&self.device, plane_id) {
                Ok(plane_type) => plane_type,
                Err(error) => {
                    issues.push(KmsPlaneIssue {
                        plane_id,
                        plane_type: None,
                        error,
                    });
                    continue;
                }
            };

            planes.push(KmsActivePlane {
                id: plane_id,
                plane_type,
                framebuffer,
            });
        }

        Ok(KmsPlaneSnapshot { planes, issues })
    }
}

fn query_plane_type(
    device: &KmsDevice,
    plane_id: plane::Handle,
) -> Result<PlaneType, KmsPlaneCaptureError> {
    let properties = device
        .get_properties(plane_id)
        .map_err(KmsPlaneCaptureError::QueryProperties)?;

    for (property_id, value) in properties.iter() {
        let property = device
            .get_property(*property_id)
            .map_err(KmsPlaneCaptureError::QueryProperties)?;

        if property.name().to_bytes() != b"type" {
            continue;
        }

        return match *value {
            value if value == u64::from(PlaneType::Primary as u32) => Ok(PlaneType::Primary),
            value if value == u64::from(PlaneType::Overlay as u32) => Ok(PlaneType::Overlay),
            value if value == u64::from(PlaneType::Cursor as u32) => Ok(PlaneType::Cursor),
            value => Err(KmsPlaneCaptureError::InvalidProperty {
                property: "type",
                value,
            }),
        };
    }

    Err(KmsPlaneCaptureError::MissingProperty { property: "type" })
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
