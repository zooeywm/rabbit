use std::{fs, path::PathBuf};

use drm::control::{Device as _, PlaneType, connector, crtc, plane};
use eros::Context;

use crate::infra::platform::screen_capture::{
    device::KmsDevice,
        types::{
            KmsActivePlane, KmsDestinationRect, KmsPlaneCaptureError,
            KmsPlaneIssue, KmsPlanePlacement, KmsPlaneSnapshot,
            KmsPlaneTransform, KmsRotation, KmsSourceRect,
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

            let properties = match query_plane_properties(&self.device, plane_id) {
                Ok(properties) => properties,
                Err(error) => {
                    issues.push(KmsPlaneIssue {
                        plane_id,
                        plane_type: None,
                        error,
                    });
                    continue;
                }
            };
            let Some(placement) = properties.placement else {
                continue;
            };

            planes.push(KmsActivePlane {
                id: plane_id,
                plane_type: properties.plane_type,
                framebuffer,
                placement,
            });
        }

        planes.sort_unstable_by_key(|plane| (plane.placement.zpos, u32::from(plane.id)));

        Ok(KmsPlaneSnapshot { planes, issues })
    }
}

fn query_plane_properties(
    device: &KmsDevice,
    plane_id: plane::Handle,
) -> Result<KmsPlaneProperties, KmsPlaneCaptureError> {
    let properties = device
        .get_properties(plane_id)
        .map_err(KmsPlaneCaptureError::QueryProperties)?;
    let mut values = RawPlaneProperties::default();

    for (property_id, value) in properties.iter() {
        let property = device
            .get_property(*property_id)
            .map_err(KmsPlaneCaptureError::QueryProperties)?;

        match property.name().to_bytes() {
            b"type" => values.plane_type = Some(*value),
            b"zpos" => values.zpos = Some(*value),
            b"SRC_X" => values.source_x = Some(*value),
            b"SRC_Y" => values.source_y = Some(*value),
            b"SRC_W" => values.source_width = Some(*value),
            b"SRC_H" => values.source_height = Some(*value),
            b"CRTC_X" => values.destination_x = Some(*value),
            b"CRTC_Y" => values.destination_y = Some(*value),
            b"CRTC_W" => values.destination_width = Some(*value),
            b"CRTC_H" => values.destination_height = Some(*value),
            b"rotation" => values.rotation = Some(*value),
            _ => {}
        }
    }

    values.try_into()
}

#[derive(Debug, Default)]
struct RawPlaneProperties {
    plane_type: Option<u64>,
    zpos: Option<u64>,
    source_x: Option<u64>,
    source_y: Option<u64>,
    source_width: Option<u64>,
    source_height: Option<u64>,
    destination_x: Option<u64>,
    destination_y: Option<u64>,
    destination_width: Option<u64>,
    destination_height: Option<u64>,
    rotation: Option<u64>,
}

struct KmsPlaneProperties {
    plane_type: PlaneType,
    placement: Option<KmsPlanePlacement>,
}

impl TryFrom<RawPlaneProperties> for KmsPlaneProperties {
    type Error = KmsPlaneCaptureError;

    fn try_from(values: RawPlaneProperties) -> Result<Self, Self::Error> {
        let plane_type = required(values.plane_type, "type")?;
        let plane_type = match plane_type {
            value if value == u64::from(PlaneType::Primary as u32) => PlaneType::Primary,
            value if value == u64::from(PlaneType::Overlay as u32) => PlaneType::Overlay,
            value if value == u64::from(PlaneType::Cursor as u32) => PlaneType::Cursor,
            value => {
                return Err(KmsPlaneCaptureError::InvalidProperty {
                    property: "type",
                    value,
                });
            }
        };
        let zpos = required(values.zpos, "zpos")?;
        let source = KmsSourceRect {
            x: unsigned_32(values.source_x, "SRC_X")?,
            y: unsigned_32(values.source_y, "SRC_Y")?,
            width: unsigned_32(values.source_width, "SRC_W")?,
            height: unsigned_32(values.source_height, "SRC_H")?,
        };
        let destination = KmsDestinationRect {
            x: signed_32(values.destination_x, "CRTC_X")?,
            y: signed_32(values.destination_y, "CRTC_Y")?,
            width: unsigned_32(values.destination_width, "CRTC_W")?,
            height: unsigned_32(values.destination_height, "CRTC_H")?,
        };
        let transform = match values.rotation {
            Some(rotation) => KmsPlaneTransform::try_from(rotation)?,
            None => KmsPlaneTransform::default(),
        };

        let placement = if source.width == 0
            || source.height == 0
            || destination.width == 0
            || destination.height == 0
        {
            None
        } else {
            Some(KmsPlanePlacement {
                zpos,
                source,
                destination,
                transform,
            })
        };

        Ok(Self {
            plane_type,
            placement,
        })
    }
}

impl TryFrom<u64> for KmsPlaneTransform {
    type Error = KmsPlaneCaptureError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let rotation_mask = KmsTransformFlag::Rotate0 as u64
            | KmsTransformFlag::Rotate90 as u64
            | KmsTransformFlag::Rotate180 as u64
            | KmsTransformFlag::Rotate270 as u64;
        let reflection_mask =
            KmsTransformFlag::ReflectX as u64 | KmsTransformFlag::ReflectY as u64;

        if value & !(rotation_mask | reflection_mask) != 0 {
            return Err(KmsPlaneCaptureError::InvalidProperty {
                property: "rotation",
                value,
            });
        }

        let rotation = match value & rotation_mask {
            flag if flag == KmsTransformFlag::Rotate0 as u64 => KmsRotation::Rotate0,
            flag if flag == KmsTransformFlag::Rotate90 as u64 => KmsRotation::Rotate90,
            flag if flag == KmsTransformFlag::Rotate180 as u64 => KmsRotation::Rotate180,
            flag if flag == KmsTransformFlag::Rotate270 as u64 => KmsRotation::Rotate270,
            _ => {
                return Err(KmsPlaneCaptureError::InvalidProperty {
                    property: "rotation",
                    value,
                });
            }
        };

        Ok(Self {
            rotation,
            reflect_x: value & KmsTransformFlag::ReflectX as u64 != 0,
            reflect_y: value & KmsTransformFlag::ReflectY as u64 != 0,
        })
    }
}

#[repr(u64)]
enum KmsTransformFlag {
    Rotate0 = 1 << 0,
    Rotate90 = 1 << 1,
    Rotate180 = 1 << 2,
    Rotate270 = 1 << 3,
    ReflectX = 1 << 4,
    ReflectY = 1 << 5,
}

fn required(
    value: Option<u64>,
    property: &'static str,
) -> Result<u64, KmsPlaneCaptureError> {
    value.ok_or(KmsPlaneCaptureError::MissingProperty { property })
}

fn unsigned_32(
    value: Option<u64>,
    property: &'static str,
) -> Result<u32, KmsPlaneCaptureError> {
    let value = required(value, property)?;

    u32::try_from(value).map_err(|_| KmsPlaneCaptureError::InvalidProperty { property, value })
}

fn signed_32(
    value: Option<u64>,
    property: &'static str,
) -> Result<i32, KmsPlaneCaptureError> {
    let value = required(value, property)?;

    i32::try_from(value as i64)
        .map_err(|_| KmsPlaneCaptureError::InvalidProperty { property, value })
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
