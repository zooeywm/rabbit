use std::{fs, path::PathBuf};

use drm::{
    Device as _, VblankWaitFlags, VblankWaitTarget,
    control::{Device as _, PlaneType, connector, crtc, plane, property},
};
use eros::Context;
use tracing::{error, warn};

use crate::{
    infra::platform::screen_capture::kms::{
        device::KmsDevice,
        types::{
            KmsActivePlane, KmsColorEncoding, KmsColorRange, KmsCursorHotspot, KmsDestinationRect,
            KmsPixelBlendMode, KmsPlaneBlend, KmsPlaneCaptureError, KmsPlaneColor, KmsPlaneIssue,
            KmsPlanePlacement, KmsPlaneSnapshot, KmsPlaneTransform, KmsRotation, KmsSourceRect,
        },
    },
    kernel::geometry::PixelSize,
};

const DRM_CLASS_PATH: &str = "/sys/class/drm";
const DRM_DEVICE_PATH: &str = "/dev/dri";

#[derive(Debug)]
pub(crate) struct KmsOutput {
    pub device: KmsDevice,
    pub connector: connector::Handle,
    pub crtc: crtc::Handle,
    vblank_crtc_index: u32,
    planes: Vec<KmsPlane>,
}

#[derive(Debug)]
struct KmsPlane {
    id: plane::Handle,
    plane_type: PlaneType,
    properties: Vec<KmsPlaneProperty>,
}

#[derive(Debug)]
struct KmsPlaneProperty {
    handle: property::Handle,
    kind: KmsPlanePropertyKind,
}

#[derive(Debug)]
enum KmsPlanePropertyKind {
    Zpos,
    SourceX,
    SourceY,
    SourceWidth,
    SourceHeight,
    DestinationX,
    DestinationY,
    DestinationWidth,
    DestinationHeight,
    Rotation,
    Alpha,
    PixelBlendMode(property::Info),
    ColorEncoding(property::Info),
    ColorRange(property::Info),
    HotspotX,
    HotspotY,
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
            let Some((connector, crtc, vblank_crtc_index)) =
                device.find_active_output(screen_name)?
            else {
                continue;
            };

            if output.is_some() {
                eros::bail!("More than one active DRM connector matches screen {screen_name}");
            }

            let planes = discover_planes(&device, crtc)?;
            output = Some(Self {
                device,
                connector,
                crtc,
                vblank_crtc_index,
                planes,
            });
        }

        Ok(output
            .with_context(|| format!("No active DRM connector matches screen {screen_name}"))?)
    }

    pub(crate) fn wait_for_vblank(&self) -> eros::Result<()> {
        self.device
            .wait_vblank(
                VblankWaitTarget::Relative(1),
                VblankWaitFlags::empty(),
                self.vblank_crtc_index,
                0,
            )
            .with_context(|| {
                format!(
                    "Failed to wait for DRM CRTC {:?} vblank on {}",
                    self.crtc,
                    self.device.path().display()
                )
            })?;

        Ok(())
    }

    pub(crate) fn snapshot_planes(&self) -> eros::Result<KmsPlaneSnapshot> {
        let crtc = self.device.get_crtc(self.crtc).with_context(|| {
            format!(
                "Failed to query DRM CRTC {:?} on {}",
                self.crtc,
                self.device.path().display()
            )
        })?;
        let mode = crtc.mode().with_context(|| {
            format!(
                "DRM CRTC {:?} on {} has no active mode",
                self.crtc,
                self.device.path().display()
            )
        })?;
        let (width, height) = mode.size();
        let output_size = PixelSize {
            width: u32::from(width),
            height: u32::from(height),
        };
        let mut planes = Vec::new();
        let mut issues = Vec::new();

        for known_plane in &self.planes {
            let plane = match self.device.get_plane(known_plane.id) {
                Ok(plane) => plane,
                Err(source) => {
                    issues.push(KmsPlaneIssue {
                        plane_id: known_plane.id,
                        plane_type: Some(known_plane.plane_type),
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

            let properties = match query_plane_properties(&self.device, known_plane) {
                Ok(properties) => properties,
                Err(error) => {
                    issues.push(KmsPlaneIssue {
                        plane_id: known_plane.id,
                        plane_type: Some(known_plane.plane_type),
                        error,
                    });
                    continue;
                }
            };
            let Some(placement) = properties.placement else {
                continue;
            };

            planes.push(KmsActivePlane {
                id: known_plane.id,
                plane_type: properties.plane_type,
                framebuffer,
                placement,
                blend: properties.blend,
                color: properties.color,
                cursor_hotspot: properties.cursor_hotspot,
            });
        }

        planes.sort_unstable_by_key(|plane| (plane.placement.zpos, u32::from(plane.id)));

        Ok(KmsPlaneSnapshot {
            output_size,
            planes,
            issues,
        })
    }
}

fn discover_planes(device: &KmsDevice, crtc: crtc::Handle) -> eros::Result<Vec<KmsPlane>> {
    let resources = device.resource_handles().with_context(|| {
        format!(
            "Failed to enumerate DRM resources on {}",
            device.path().display()
        )
    })?;
    let plane_handles = device.plane_handles().with_context(|| {
        format!(
            "Failed to enumerate DRM planes on {}",
            device.path().display()
        )
    })?;
    let mut planes = Vec::new();

    for plane_id in plane_handles {
        let plane = match device.get_plane(plane_id) {
            Ok(plane) => plane,
            Err(source) => {
                let error = KmsPlaneCaptureError::QueryPlane(source);
                error!(
                    ?plane_id,
                    ?crtc,
                    device = %device.path().display(),
                    error = ?error,
                    "Skipping DRM plane after discovery failed"
                );
                continue;
            }
        };

        if !resources
            .filter_crtcs(plane.possible_crtcs())
            .contains(&crtc)
        {
            continue;
        }

        let (plane_type, properties) = match discover_plane_properties(device, plane_id) {
            Ok(properties) => properties,
            Err(error) => {
                error!(
                    ?plane_id,
                    ?crtc,
                    device = %device.path().display(),
                    error = ?error,
                    "Skipping DRM plane after property metadata discovery failed"
                );
                continue;
            }
        };

        planes.push(KmsPlane {
            id: plane_id,
            plane_type,
            properties,
        });
    }

    for (plane_type, name) in [
        (PlaneType::Primary, "primary"),
        (PlaneType::Overlay, "overlay"),
        (PlaneType::Cursor, "cursor"),
    ] {
        if !planes.iter().any(|plane| plane.plane_type == plane_type) {
            warn!(
                ?crtc,
                device = %device.path().display(),
                plane_type = name,
                "DRM output does not expose this plane type; it will not be probed"
            );
        }
    }

    Ok(planes)
}

fn discover_plane_properties(
    device: &KmsDevice,
    plane_id: plane::Handle,
) -> Result<(PlaneType, Vec<KmsPlaneProperty>), KmsPlaneCaptureError> {
    let properties = device
        .get_properties(plane_id)
        .map_err(KmsPlaneCaptureError::QueryProperties)?;
    let mut discovered_plane_type = None;
    let mut cached = Vec::with_capacity(properties.as_props_and_values().0.len());

    for (property_id, value) in properties.iter() {
        let info = device
            .get_property(*property_id)
            .map_err(KmsPlaneCaptureError::QueryProperties)?;

        let kind = match info.name().to_bytes() {
            b"type" => {
                discovered_plane_type = Some(plane_type(*value)?);
                continue;
            }
            b"zpos" => KmsPlanePropertyKind::Zpos,
            b"SRC_X" => KmsPlanePropertyKind::SourceX,
            b"SRC_Y" => KmsPlanePropertyKind::SourceY,
            b"SRC_W" => KmsPlanePropertyKind::SourceWidth,
            b"SRC_H" => KmsPlanePropertyKind::SourceHeight,
            b"CRTC_X" => KmsPlanePropertyKind::DestinationX,
            b"CRTC_Y" => KmsPlanePropertyKind::DestinationY,
            b"CRTC_W" => KmsPlanePropertyKind::DestinationWidth,
            b"CRTC_H" => KmsPlanePropertyKind::DestinationHeight,
            b"rotation" => KmsPlanePropertyKind::Rotation,
            b"alpha" => KmsPlanePropertyKind::Alpha,
            b"pixel blend mode" => KmsPlanePropertyKind::PixelBlendMode(info),
            b"COLOR_ENCODING" => KmsPlanePropertyKind::ColorEncoding(info),
            b"COLOR_RANGE" => KmsPlanePropertyKind::ColorRange(info),
            b"HOTSPOT_X" => KmsPlanePropertyKind::HotspotX,
            b"HOTSPOT_Y" => KmsPlanePropertyKind::HotspotY,
            _ => continue,
        };
        cached.push(KmsPlaneProperty {
            handle: *property_id,
            kind,
        });
    }

    Ok((
        discovered_plane_type.ok_or(KmsPlaneCaptureError::MissingProperty { property: "type" })?,
        cached,
    ))
}

fn query_plane_properties(
    device: &KmsDevice,
    plane: &KmsPlane,
) -> Result<KmsPlaneProperties, KmsPlaneCaptureError> {
    let properties = device
        .get_properties(plane.id)
        .map_err(KmsPlaneCaptureError::QueryProperties)?;
    let mut values = RawPlaneProperties {
        plane_type: Some(u64::from(plane.plane_type as u32)),
        ..RawPlaneProperties::default()
    };

    for (property_id, value) in properties.iter() {
        let Some(property) = plane
            .properties
            .iter()
            .find(|property| property.handle == *property_id)
        else {
            continue;
        };

        match &property.kind {
            KmsPlanePropertyKind::Zpos => values.zpos = Some(*value),
            KmsPlanePropertyKind::SourceX => values.source_x = Some(*value),
            KmsPlanePropertyKind::SourceY => values.source_y = Some(*value),
            KmsPlanePropertyKind::SourceWidth => values.source_width = Some(*value),
            KmsPlanePropertyKind::SourceHeight => values.source_height = Some(*value),
            KmsPlanePropertyKind::DestinationX => values.destination_x = Some(*value),
            KmsPlanePropertyKind::DestinationY => values.destination_y = Some(*value),
            KmsPlanePropertyKind::DestinationWidth => values.destination_width = Some(*value),
            KmsPlanePropertyKind::DestinationHeight => values.destination_height = Some(*value),
            KmsPlanePropertyKind::Rotation => values.rotation = Some(*value),
            KmsPlanePropertyKind::Alpha => values.alpha = Some(*value),
            KmsPlanePropertyKind::PixelBlendMode(info) => {
                values.pixel_blend_mode = Some(pixel_blend_mode(info, *value)?);
            }
            KmsPlanePropertyKind::ColorEncoding(info) => {
                values.color_encoding = Some(color_encoding(info, *value)?);
            }
            KmsPlanePropertyKind::ColorRange(info) => {
                values.color_range = Some(color_range(info, *value)?);
            }
            KmsPlanePropertyKind::HotspotX => values.hotspot_x = Some(*value),
            KmsPlanePropertyKind::HotspotY => values.hotspot_y = Some(*value),
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
    alpha: Option<u64>,
    pixel_blend_mode: Option<KmsPixelBlendMode>,
    color_encoding: Option<KmsColorEncoding>,
    color_range: Option<KmsColorRange>,
    hotspot_x: Option<u64>,
    hotspot_y: Option<u64>,
}

struct KmsPlaneProperties {
    plane_type: PlaneType,
    placement: Option<KmsPlanePlacement>,
    blend: KmsPlaneBlend,
    color: KmsPlaneColor,
    cursor_hotspot: Option<KmsCursorHotspot>,
}

impl TryFrom<RawPlaneProperties> for KmsPlaneProperties {
    type Error = KmsPlaneCaptureError;

    fn try_from(values: RawPlaneProperties) -> Result<Self, Self::Error> {
        let plane_type = plane_type(required(values.plane_type, "type")?)?;
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
        let alpha = match values.alpha {
            Some(alpha) => {
                u16::try_from(alpha).map_err(|_| KmsPlaneCaptureError::InvalidProperty {
                    property: "alpha",
                    value: alpha,
                })?
            }
            None => u16::MAX,
        };
        let blend = KmsPlaneBlend {
            alpha,
            pixel_mode: values.pixel_blend_mode.unwrap_or_default(),
        };
        let color = match (values.color_encoding, values.color_range) {
            (Some(encoding), Some(range)) => KmsPlaneColor { encoding, range },
            (None, None) => KmsPlaneColor::default(),
            (Some(_), None) => {
                return Err(KmsPlaneCaptureError::MissingProperty {
                    property: "COLOR_RANGE",
                });
            }
            (None, Some(_)) => {
                return Err(KmsPlaneCaptureError::MissingProperty {
                    property: "COLOR_ENCODING",
                });
            }
        };
        let cursor_hotspot = match plane_type {
            PlaneType::Cursor => match (values.hotspot_x, values.hotspot_y) {
                (Some(x), Some(y)) => Some(KmsCursorHotspot {
                    x: u32::try_from(x).map_err(|_| KmsPlaneCaptureError::InvalidProperty {
                        property: "HOTSPOT_X",
                        value: x,
                    })?,
                    y: u32::try_from(y).map_err(|_| KmsPlaneCaptureError::InvalidProperty {
                        property: "HOTSPOT_Y",
                        value: y,
                    })?,
                }),
                (None, None) => Some(KmsCursorHotspot::default()),
                (Some(_), None) => {
                    return Err(KmsPlaneCaptureError::MissingProperty {
                        property: "HOTSPOT_Y",
                    });
                }
                (None, Some(_)) => {
                    return Err(KmsPlaneCaptureError::MissingProperty {
                        property: "HOTSPOT_X",
                    });
                }
            },
            PlaneType::Primary | PlaneType::Overlay => None,
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
            blend,
            color,
            cursor_hotspot,
        })
    }
}

fn plane_type(value: u64) -> Result<PlaneType, KmsPlaneCaptureError> {
    match value {
        value if value == u64::from(PlaneType::Primary as u32) => Ok(PlaneType::Primary),
        value if value == u64::from(PlaneType::Overlay as u32) => Ok(PlaneType::Overlay),
        value if value == u64::from(PlaneType::Cursor as u32) => Ok(PlaneType::Cursor),
        value => Err(KmsPlaneCaptureError::InvalidProperty {
            property: "type",
            value,
        }),
    }
}

fn pixel_blend_mode(
    property: &drm::control::property::Info,
    value: u64,
) -> Result<KmsPixelBlendMode, KmsPlaneCaptureError> {
    let converted = property.value_type().convert_value(value);
    let Some(value_name) = converted.as_enum() else {
        return Err(KmsPlaneCaptureError::InvalidProperty {
            property: "pixel blend mode",
            value,
        });
    };

    match value_name.name().to_bytes() {
        b"None" => Ok(KmsPixelBlendMode::None),
        b"Pre-multiplied" => Ok(KmsPixelBlendMode::PreMultiplied),
        b"Coverage" => Ok(KmsPixelBlendMode::Coverage),
        _ => Err(KmsPlaneCaptureError::InvalidProperty {
            property: "pixel blend mode",
            value,
        }),
    }
}

fn color_encoding(
    property: &drm::control::property::Info,
    value: u64,
) -> Result<KmsColorEncoding, KmsPlaneCaptureError> {
    let converted = property.value_type().convert_value(value);
    let Some(value_name) = converted.as_enum() else {
        return Err(KmsPlaneCaptureError::InvalidProperty {
            property: "COLOR_ENCODING",
            value,
        });
    };

    match value_name.name().to_bytes() {
        b"ITU-R BT.601 YCbCr" => Ok(KmsColorEncoding::Bt601),
        b"ITU-R BT.709 YCbCr" => Ok(KmsColorEncoding::Bt709),
        b"ITU-R BT.2020 YCbCr" => Ok(KmsColorEncoding::Bt2020),
        _ => Err(KmsPlaneCaptureError::InvalidProperty {
            property: "COLOR_ENCODING",
            value,
        }),
    }
}

fn color_range(
    property: &drm::control::property::Info,
    value: u64,
) -> Result<KmsColorRange, KmsPlaneCaptureError> {
    let converted = property.value_type().convert_value(value);
    let Some(value_name) = converted.as_enum() else {
        return Err(KmsPlaneCaptureError::InvalidProperty {
            property: "COLOR_RANGE",
            value,
        });
    };

    match value_name.name().to_bytes() {
        b"YCbCr limited range" => Ok(KmsColorRange::Limited),
        b"YCbCr full range" => Ok(KmsColorRange::Full),
        _ => Err(KmsPlaneCaptureError::InvalidProperty {
            property: "COLOR_RANGE",
            value,
        }),
    }
}

impl TryFrom<u64> for KmsPlaneTransform {
    type Error = KmsPlaneCaptureError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let rotation_mask = KmsTransformFlag::Rotate0 as u64
            | KmsTransformFlag::Rotate90 as u64
            | KmsTransformFlag::Rotate180 as u64
            | KmsTransformFlag::Rotate270 as u64;
        let reflection_mask = KmsTransformFlag::ReflectX as u64 | KmsTransformFlag::ReflectY as u64;

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

fn required(value: Option<u64>, property: &'static str) -> Result<u64, KmsPlaneCaptureError> {
    value.ok_or(KmsPlaneCaptureError::MissingProperty { property })
}

fn unsigned_32(value: Option<u64>, property: &'static str) -> Result<u32, KmsPlaneCaptureError> {
    let value = required(value, property)?;

    u32::try_from(value).map_err(|_| KmsPlaneCaptureError::InvalidProperty { property, value })
}

fn signed_32(value: Option<u64>, property: &'static str) -> Result<i32, KmsPlaneCaptureError> {
    let value = required(value, property)?;

    i32::try_from(value as i64)
        .map_err(|_| KmsPlaneCaptureError::InvalidProperty { property, value })
}

fn device_paths(screen_name: &str) -> eros::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(DRM_CLASS_PATH)
        .with_context(|| format!("Failed to enumerate {DRM_CLASS_PATH}"))?;
    let mut paths = Vec::new();

    for entry in entries {
        let entry =
            entry.with_context(|| format!("Failed to read an entry in {DRM_CLASS_PATH}"))?;
        let entry_name = entry.file_name();
        let entry_name = entry_name
            .to_str()
            .with_context(|| format!("DRM class entry name {entry_name:?} is not valid UTF-8"))?;
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
