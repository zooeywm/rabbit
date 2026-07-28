use crate::kernel::input::{KeyboardKey, MouseButton};

pub(super) fn keyboard_key_from_slint(text: &str) -> Option<KeyboardKey> {
    let mut characters = text.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }

    use slint::platform::Key;
    let special = [
        (Key::Backspace, KeyboardKey::BACKSPACE),
        (Key::Tab, KeyboardKey::TAB),
        (Key::Backtab, KeyboardKey::TAB),
        (Key::Return, KeyboardKey::ENTER),
        (Key::Escape, KeyboardKey::ESCAPE),
        (Key::Delete, KeyboardKey::DELETE),
        (Key::Shift, KeyboardKey::SHIFT_LEFT),
        (Key::ShiftR, KeyboardKey::SHIFT_RIGHT),
        (Key::Control, KeyboardKey::CONTROL_LEFT),
        (Key::ControlR, KeyboardKey::CONTROL_RIGHT),
        (Key::Alt, KeyboardKey::ALT_LEFT),
        (Key::AltGr, KeyboardKey::ALT_RIGHT),
        (Key::Meta, KeyboardKey::META_LEFT),
        (Key::MetaR, KeyboardKey::META_RIGHT),
        (Key::CapsLock, KeyboardKey::CAPS_LOCK),
        (Key::UpArrow, KeyboardKey::ARROW_UP),
        (Key::DownArrow, KeyboardKey::ARROW_DOWN),
        (Key::LeftArrow, KeyboardKey::ARROW_LEFT),
        (Key::RightArrow, KeyboardKey::ARROW_RIGHT),
        (Key::Insert, KeyboardKey::INSERT),
        (Key::Home, KeyboardKey::HOME),
        (Key::End, KeyboardKey::END),
        (Key::PageUp, KeyboardKey::PAGE_UP),
        (Key::PageDown, KeyboardKey::PAGE_DOWN),
        (Key::ScrollLock, KeyboardKey::SCROLL_LOCK),
        (Key::Pause, KeyboardKey::PAUSE),
        (Key::SysReq, KeyboardKey::PRINT_SCREEN),
        (Key::Menu, KeyboardKey::MENU),
        (Key::F1, KeyboardKey::F1),
        (Key::F2, KeyboardKey::F2),
        (Key::F3, KeyboardKey::F3),
        (Key::F4, KeyboardKey::F4),
        (Key::F5, KeyboardKey::F5),
        (Key::F6, KeyboardKey::F6),
        (Key::F7, KeyboardKey::F7),
        (Key::F8, KeyboardKey::F8),
        (Key::F9, KeyboardKey::F9),
        (Key::F10, KeyboardKey::F10),
        (Key::F11, KeyboardKey::F11),
        (Key::F12, KeyboardKey::F12),
        (Key::F13, KeyboardKey::F13),
        (Key::F14, KeyboardKey::F14),
        (Key::F15, KeyboardKey::F15),
        (Key::F16, KeyboardKey::F16),
        (Key::F17, KeyboardKey::F17),
        (Key::F18, KeyboardKey::F18),
        (Key::F19, KeyboardKey::F19),
        (Key::F20, KeyboardKey::F20),
        (Key::F21, KeyboardKey::F21),
        (Key::F22, KeyboardKey::F22),
        (Key::F23, KeyboardKey::F23),
        (Key::F24, KeyboardKey::F24),
    ];
    if let Some((_, key)) = special
        .into_iter()
        .find(|(slint_key, _)| char::from(*slint_key) == character)
    {
        return Some(key);
    }

    Some(match character.to_ascii_lowercase() {
        'a' => KeyboardKey::A,
        'b' => KeyboardKey::B,
        'c' => KeyboardKey::C,
        'd' => KeyboardKey::D,
        'e' => KeyboardKey::E,
        'f' => KeyboardKey::F,
        'g' => KeyboardKey::G,
        'h' => KeyboardKey::H,
        'i' => KeyboardKey::I,
        'j' => KeyboardKey::J,
        'k' => KeyboardKey::K,
        'l' => KeyboardKey::L,
        'm' => KeyboardKey::M,
        'n' => KeyboardKey::N,
        'o' => KeyboardKey::O,
        'p' => KeyboardKey::P,
        'q' => KeyboardKey::Q,
        'r' => KeyboardKey::R,
        's' => KeyboardKey::S,
        't' => KeyboardKey::T,
        'u' => KeyboardKey::U,
        'v' => KeyboardKey::V,
        'w' => KeyboardKey::W,
        'x' => KeyboardKey::X,
        'y' => KeyboardKey::Y,
        'z' => KeyboardKey::Z,
        '0' | ')' => KeyboardKey::DIGIT_0,
        '1' | '!' => KeyboardKey::DIGIT_1,
        '2' | '@' => KeyboardKey::DIGIT_2,
        '3' | '#' => KeyboardKey::DIGIT_3,
        '4' | '$' => KeyboardKey::DIGIT_4,
        '5' | '%' => KeyboardKey::DIGIT_5,
        '6' | '^' => KeyboardKey::DIGIT_6,
        '7' | '&' => KeyboardKey::DIGIT_7,
        '8' | '*' => KeyboardKey::DIGIT_8,
        '9' | '(' => KeyboardKey::DIGIT_9,
        ' ' => KeyboardKey::SPACE,
        '-' | '_' => KeyboardKey::MINUS,
        '=' | '+' => KeyboardKey::EQUAL,
        '[' | '{' => KeyboardKey::LEFT_BRACKET,
        ']' | '}' => KeyboardKey::RIGHT_BRACKET,
        '\\' | '|' => KeyboardKey::BACKSLASH,
        ';' | ':' => KeyboardKey::SEMICOLON,
        '\'' | '"' => KeyboardKey::QUOTE,
        '`' | '~' => KeyboardKey::BACKQUOTE,
        ',' | '<' => KeyboardKey::COMMA,
        '.' | '>' => KeyboardKey::PERIOD,
        '/' | '?' => KeyboardKey::SLASH,
        _ => return None,
    })
}

pub(super) const fn mouse_button_from_slint(
    button: slint::platform::PointerEventButton,
) -> Option<MouseButton> {
    use slint::platform::PointerEventButton;
    match button {
        PointerEventButton::Left => Some(MouseButton::Left),
        PointerEventButton::Right => Some(MouseButton::Right),
        PointerEventButton::Middle => Some(MouseButton::Middle),
        PointerEventButton::Back => Some(MouseButton::Back),
        PointerEventButton::Forward => Some(MouseButton::Forward),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub(super) struct RelativeDeltaAccumulator {
    x: f32,
    y: f32,
}

impl RelativeDeltaAccumulator {
    pub(super) fn accumulate(&mut self, delta_x: f32, delta_y: f32) -> Option<(i32, i32)> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return None;
        }
        self.x += delta_x;
        self.y += delta_y;
        let x = self.x.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        let y = self.y.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        self.x -= x as f32;
        self.y -= y as f32;
        (x != 0 || y != 0).then_some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_slint_text_to_rabbit_base_keys() {
        assert_eq!(keyboard_key_from_slint("A"), Some(KeyboardKey::A));
        assert_eq!(keyboard_key_from_slint("!"), Some(KeyboardKey::DIGIT_1));
        assert_eq!(
            keyboard_key_from_slint(&char::from(slint::platform::Key::F12).to_string()),
            Some(KeyboardKey::F12)
        );
        assert_eq!(keyboard_key_from_slint("文"), None);
    }

    #[test]
    fn relative_delta_rejects_empty_and_non_finite_motion() {
        let mut accumulator = RelativeDeltaAccumulator::default();
        assert_eq!(accumulator.accumulate(0.4, -0.4), None);
        assert_eq!(accumulator.accumulate(0.4, -0.4), Some((1, -1)));
        assert_eq!(accumulator.accumulate(f32::NAN, 1.0), None);
    }
}
