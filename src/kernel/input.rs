//! Stable remote-input protocol values and platform injection port.

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

/// Rabbit-owned keyboard code. Values are stable wire protocol identifiers,
/// not Linux input codes or Windows virtual-key values.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardKey(u16);

impl KeyboardKey {
    pub const A: Self = Self(b'a' as u16);
    pub const B: Self = Self(b'b' as u16);
    pub const C: Self = Self(b'c' as u16);
    pub const D: Self = Self(b'd' as u16);
    pub const E: Self = Self(b'e' as u16);
    pub const F: Self = Self(b'f' as u16);
    pub const G: Self = Self(b'g' as u16);
    pub const H: Self = Self(b'h' as u16);
    pub const I: Self = Self(b'i' as u16);
    pub const J: Self = Self(b'j' as u16);
    pub const K: Self = Self(b'k' as u16);
    pub const L: Self = Self(b'l' as u16);
    pub const M: Self = Self(b'm' as u16);
    pub const N: Self = Self(b'n' as u16);
    pub const O: Self = Self(b'o' as u16);
    pub const P: Self = Self(b'p' as u16);
    pub const Q: Self = Self(b'q' as u16);
    pub const R: Self = Self(b'r' as u16);
    pub const S: Self = Self(b's' as u16);
    pub const T: Self = Self(b't' as u16);
    pub const U: Self = Self(b'u' as u16);
    pub const V: Self = Self(b'v' as u16);
    pub const W: Self = Self(b'w' as u16);
    pub const X: Self = Self(b'x' as u16);
    pub const Y: Self = Self(b'y' as u16);
    pub const Z: Self = Self(b'z' as u16);
    pub const DIGIT_0: Self = Self(b'0' as u16);
    pub const DIGIT_1: Self = Self(b'1' as u16);
    pub const DIGIT_2: Self = Self(b'2' as u16);
    pub const DIGIT_3: Self = Self(b'3' as u16);
    pub const DIGIT_4: Self = Self(b'4' as u16);
    pub const DIGIT_5: Self = Self(b'5' as u16);
    pub const DIGIT_6: Self = Self(b'6' as u16);
    pub const DIGIT_7: Self = Self(b'7' as u16);
    pub const DIGIT_8: Self = Self(b'8' as u16);
    pub const DIGIT_9: Self = Self(b'9' as u16);
    pub const SPACE: Self = Self(b' ' as u16);
    pub const MINUS: Self = Self(b'-' as u16);
    pub const EQUAL: Self = Self(b'=' as u16);
    pub const LEFT_BRACKET: Self = Self(b'[' as u16);
    pub const RIGHT_BRACKET: Self = Self(b']' as u16);
    pub const BACKSLASH: Self = Self(b'\\' as u16);
    pub const SEMICOLON: Self = Self(b';' as u16);
    pub const QUOTE: Self = Self(b'\'' as u16);
    pub const BACKQUOTE: Self = Self(b'`' as u16);
    pub const COMMA: Self = Self(b',' as u16);
    pub const PERIOD: Self = Self(b'.' as u16);
    pub const SLASH: Self = Self(b'/' as u16);

    pub const ESCAPE: Self = Self(0x100);
    pub const TAB: Self = Self(0x101);
    pub const ENTER: Self = Self(0x102);
    pub const BACKSPACE: Self = Self(0x103);
    pub const DELETE: Self = Self(0x104);
    pub const INSERT: Self = Self(0x105);
    pub const HOME: Self = Self(0x106);
    pub const END: Self = Self(0x107);
    pub const PAGE_UP: Self = Self(0x108);
    pub const PAGE_DOWN: Self = Self(0x109);
    pub const ARROW_UP: Self = Self(0x10a);
    pub const ARROW_DOWN: Self = Self(0x10b);
    pub const ARROW_LEFT: Self = Self(0x10c);
    pub const ARROW_RIGHT: Self = Self(0x10d);
    pub const SHIFT_LEFT: Self = Self(0x10e);
    pub const SHIFT_RIGHT: Self = Self(0x10f);
    pub const CONTROL_LEFT: Self = Self(0x110);
    pub const CONTROL_RIGHT: Self = Self(0x111);
    pub const ALT_LEFT: Self = Self(0x112);
    pub const ALT_RIGHT: Self = Self(0x113);
    pub const META_LEFT: Self = Self(0x114);
    pub const META_RIGHT: Self = Self(0x115);
    pub const CAPS_LOCK: Self = Self(0x116);
    pub const SCROLL_LOCK: Self = Self(0x117);
    pub const PAUSE: Self = Self(0x118);
    pub const PRINT_SCREEN: Self = Self(0x119);
    pub const MENU: Self = Self(0x11a);
    pub const F1: Self = Self(0x120);
    pub const F2: Self = Self(0x121);
    pub const F3: Self = Self(0x122);
    pub const F4: Self = Self(0x123);
    pub const F5: Self = Self(0x124);
    pub const F6: Self = Self(0x125);
    pub const F7: Self = Self(0x126);
    pub const F8: Self = Self(0x127);
    pub const F9: Self = Self(0x128);
    pub const F10: Self = Self(0x129);
    pub const F11: Self = Self(0x12a);
    pub const F12: Self = Self(0x12b);
    pub const F13: Self = Self(0x12c);
    pub const F14: Self = Self(0x12d);
    pub const F15: Self = Self(0x12e);
    pub const F16: Self = Self(0x12f);
    pub const F17: Self = Self(0x130);
    pub const F18: Self = Self(0x131);
    pub const F19: Self = Self(0x132);
    pub const F20: Self = Self(0x133);
    pub const F21: Self = Self(0x134);
    pub const F22: Self = Self(0x135);
    pub const F23: Self = Self(0x136);
    pub const F24: Self = Self(0x137);

    pub const ALL: [Self; 99] = [
        Self::A,
        Self::B,
        Self::C,
        Self::D,
        Self::E,
        Self::F,
        Self::G,
        Self::H,
        Self::I,
        Self::J,
        Self::K,
        Self::L,
        Self::M,
        Self::N,
        Self::O,
        Self::P,
        Self::Q,
        Self::R,
        Self::S,
        Self::T,
        Self::U,
        Self::V,
        Self::W,
        Self::X,
        Self::Y,
        Self::Z,
        Self::DIGIT_0,
        Self::DIGIT_1,
        Self::DIGIT_2,
        Self::DIGIT_3,
        Self::DIGIT_4,
        Self::DIGIT_5,
        Self::DIGIT_6,
        Self::DIGIT_7,
        Self::DIGIT_8,
        Self::DIGIT_9,
        Self::SPACE,
        Self::MINUS,
        Self::EQUAL,
        Self::LEFT_BRACKET,
        Self::RIGHT_BRACKET,
        Self::BACKSLASH,
        Self::SEMICOLON,
        Self::QUOTE,
        Self::BACKQUOTE,
        Self::COMMA,
        Self::PERIOD,
        Self::SLASH,
        Self::ESCAPE,
        Self::TAB,
        Self::ENTER,
        Self::BACKSPACE,
        Self::DELETE,
        Self::INSERT,
        Self::HOME,
        Self::END,
        Self::PAGE_UP,
        Self::PAGE_DOWN,
        Self::ARROW_UP,
        Self::ARROW_DOWN,
        Self::ARROW_LEFT,
        Self::ARROW_RIGHT,
        Self::SHIFT_LEFT,
        Self::SHIFT_RIGHT,
        Self::CONTROL_LEFT,
        Self::CONTROL_RIGHT,
        Self::ALT_LEFT,
        Self::ALT_RIGHT,
        Self::META_LEFT,
        Self::META_RIGHT,
        Self::CAPS_LOCK,
        Self::SCROLL_LOCK,
        Self::PAUSE,
        Self::PRINT_SCREEN,
        Self::MENU,
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
        Self::F13,
        Self::F14,
        Self::F15,
        Self::F16,
        Self::F17,
        Self::F18,
        Self::F19,
        Self::F20,
        Self::F21,
        Self::F22,
        Self::F23,
        Self::F24,
    ];

    pub const fn wire_code(self) -> u16 {
        self.0
    }
}

impl std::fmt::Debug for KeyboardKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "KeyboardKey({:#x})", self.0)
    }
}

impl TryFrom<u16> for KeyboardKey {
    type Error = eros::ErrorUnion;

    fn try_from(code: u16) -> eros::Result<Self> {
        let key = Self(code);
        if Self::ALL.contains(&key) {
            Ok(key)
        } else {
            eros::bail!("Unknown Rabbit keyboard key code {code:#x}")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputState {
    Pressed,
    Released,
}

impl InputState {
    pub const fn wire_code(self) -> u8 {
        match self {
            Self::Pressed => 0,
            Self::Released => 1,
        }
    }
}

impl TryFrom<u8> for InputState {
    type Error = eros::ErrorUnion;

    fn try_from(code: u8) -> eros::Result<Self> {
        match code {
            0 => Ok(Self::Pressed),
            1 => Ok(Self::Released),
            _ => eros::bail!("Unknown Rabbit input state {code}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardInput {
    pub screen_id: ScreenId,
    pub key: KeyboardKey,
    pub state: InputState,
    pub repeat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl MouseButton {
    pub const fn wire_code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
            Self::Back => 3,
            Self::Forward => 4,
        }
    }
}

impl TryFrom<u8> for MouseButton {
    type Error = eros::ErrorUnion;

    fn try_from(code: u8) -> eros::Result<Self> {
        match code {
            0 => Ok(Self::Left),
            1 => Ok(Self::Right),
            2 => Ok(Self::Middle),
            3 => Ok(Self::Back),
            4 => Ok(Self::Forward),
            _ => eros::bail!("Unknown Rabbit mouse button {code}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MouseButtonInput {
    pub screen_id: ScreenId,
    pub button: MouseButton,
    pub state: InputState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelativePointerMove {
    pub screen_id: ScreenId,
    pub delta_x: i32,
    pub delta_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteInputEvent {
    AbsolutePointerMove(AbsolutePointerMove),
    Keyboard(KeyboardInput),
    MouseButton(MouseButtonInput),
    RelativePointerMove(RelativePointerMove),
}

impl RemoteInputEvent {
    pub const fn screen_id(self) -> ScreenId {
        match self {
            Self::AbsolutePointerMove(input) => input.screen_id,
            Self::Keyboard(input) => input.screen_id,
            Self::MouseButton(input) => input.screen_id,
            Self::RelativePointerMove(input) => input.screen_id,
        }
    }

    pub const fn is_reliable(self) -> bool {
        !matches!(self, Self::AbsolutePointerMove(_))
    }
}

/// Platform adapter that injects remote input on the host.
pub trait RemoteInputInjector {
    fn inject(
        &mut self,
        input: RemoteInputEvent,
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
    fn rabbit_input_codes_are_stable_and_reject_unknown_values() {
        assert_eq!(KeyboardKey::A.wire_code(), u16::from(b'a'));
        assert_eq!(KeyboardKey::F1.wire_code(), 0x120);
        assert_eq!(
            KeyboardKey::try_from(KeyboardKey::F24.wire_code()).unwrap(),
            KeyboardKey::F24
        );
        assert!(KeyboardKey::try_from(0xffff).is_err());
        assert_eq!(MouseButton::Back.wire_code(), 3);
        assert!(MouseButton::try_from(5).is_err());
    }

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
    fn fill_window_mapping_uses_the_whole_window_input_surface() {
        let video = PixelSize {
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            map_viewport_position(500.0, 340.0, 1000.0, 680.0, video),
            Some(NormalizedPosition { x: 32768, y: 32768 })
        );
        assert_eq!(
            map_viewport_position(500.0, 40.0, 1000.0, 680.0, video),
            None
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
