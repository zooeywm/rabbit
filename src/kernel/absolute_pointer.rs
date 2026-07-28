//! Absolute pointer coordinates and host injection port.

use crate::kernel::{
    geometry::PixelSize,
    screen_manager::{Screen, ScreenId},
};

/// Full-range normalized coordinate used on the wire and by platform adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NormalizedPosition {
    pub x: u16,
    pub y: u16,
}

/// Absolute pointer movement relative to one streamed screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbsolutePointerMove {
    pub screen_id: ScreenId,
    pub position: NormalizedPosition,
}

/// Platform adapter that injects absolute pointer movement on the host.
pub trait AbsolutePointerInjector {
    fn move_absolute(
        &mut self,
        movement: AbsolutePointerMove,
        screen: &Screen,
        screens: &[Screen],
    ) -> eros::Result<()>;
}

/// Maps a Slint video viewport point into the fitted video rectangle.
///
/// Points in aspect-ratio letterboxing are ignored rather than being clamped
/// onto the remote desktop edge.
pub fn map_viewport_position(
    x: f32,
    y: f32,
    viewport_width: f32,
    viewport_height: f32,
    video_size: PixelSize,
) -> Option<NormalizedPosition> {
    if !x.is_finite()
        || !y.is_finite()
        || !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width <= 0.0
        || viewport_height <= 0.0
        || video_size.width == 0
        || video_size.height == 0
    {
        return None;
    }

    let video_width = video_size.width as f32;
    let video_height = video_size.height as f32;
    let scale = (viewport_width / video_width).min(viewport_height / video_height);
    let fitted_width = video_width * scale;
    let fitted_height = video_height * scale;
    let left = (viewport_width - fitted_width) * 0.5;
    let top = (viewport_height - fitted_height) * 0.5;

    if x < left || y < top || x > left + fitted_width || y > top + fitted_height {
        return None;
    }

    Some(NormalizedPosition {
        x: normalize_axis(x - left, fitted_width),
        y: normalize_axis(y - top, fitted_height),
    })
}

/// Converts a screen-local normalized position to the normalized desktop
/// bounding rectangle used by Linux uinput.
pub fn map_screen_to_desktop(
    position: NormalizedPosition,
    screen: &Screen,
    screens: &[Screen],
) -> NormalizedPosition {
    let desktop_width = screens
        .iter()
        .map(|candidate| {
            candidate
                .layout
                .rect
                .x
                .saturating_add(candidate.layout.rect.width)
        })
        .max()
        .unwrap_or(screen.layout.rect.width)
        .max(1);
    let desktop_height = screens
        .iter()
        .map(|candidate| {
            candidate
                .layout
                .rect
                .y
                .saturating_add(candidate.layout.rect.height)
        })
        .max()
        .unwrap_or(screen.layout.rect.height)
        .max(1);

    NormalizedPosition {
        x: normalize_desktop_axis(
            position.x,
            screen.layout.rect.x,
            screen.layout.rect.width,
            desktop_width,
        ),
        y: normalize_desktop_axis(
            position.y,
            screen.layout.rect.y,
            screen.layout.rect.height,
            desktop_height,
        ),
    }
}

fn normalize_axis(value: f32, length: f32) -> u16 {
    ((value / length).clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16
}

fn normalize_desktop_axis(
    local: u16,
    screen_offset: u32,
    screen_length: u32,
    desktop_length: u32,
) -> u16 {
    let local_fraction = f64::from(local) / f64::from(u16::MAX);
    let desktop_position =
        f64::from(screen_offset) + local_fraction * f64::from(screen_length.saturating_sub(1));
    let desktop_max = f64::from(desktop_length.saturating_sub(1).max(1));
    (desktop_position / desktop_max * f64::from(u16::MAX))
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        geometry::FrameRate,
        screen_manager::{ScreenLayout, ScreenRect, ScreenTransform},
    };

    #[test]
    fn viewport_mapping_ignores_letterbox_and_maps_video_edges() {
        let video = PixelSize {
            width: 1920,
            height: 1080,
        };

        assert_eq!(map_viewport_position(50.0, 0.0, 100.0, 100.0, video), None);
        assert_eq!(
            map_viewport_position(0.0, 21.875, 100.0, 100.0, video),
            Some(NormalizedPosition { x: 0, y: 0 })
        );
        assert_eq!(
            map_viewport_position(100.0, 78.125, 100.0, 100.0, video),
            Some(NormalizedPosition {
                x: u16::MAX,
                y: u16::MAX,
            })
        );
    }

    #[test]
    fn selected_screen_maps_into_virtual_desktop() {
        let screens = [screen(0, 0, 0, 1920, 1080), screen(1, 1920, 0, 1920, 1080)];
        let mapped =
            map_screen_to_desktop(NormalizedPosition { x: 0, y: 0 }, &screens[1], &screens);

        assert!((32760..=32776).contains(&mapped.x));
        assert_eq!(mapped.y, 0);
    }

    fn screen(id: u8, x: u32, y: u32, width: u32, height: u32) -> Screen {
        Screen {
            id: ScreenId(id),
            name: format!("screen-{id}"),
            resolution: PixelSize { width, height },
            frame_rate: FrameRate::new(60, 1).expect("frame rate"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x,
                    y,
                    width,
                    height,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }
}
