use eros::Context as _;
use windows::Win32::{
    Foundation::GetLastError,
    Graphics::Gdi::{GetMonitorInfoW, MONITORINFO},
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
            MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
            MOUSEEVENTF_MOVE, MOUSEEVENTF_MOVE_NOCOALESCE, MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
            MOUSEINPUT, SendInput, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        },
    },
};

use crate::kernel::{
    input::{
        InputState, KeyboardInput, KeyboardKey, MouseButton, MouseButtonInput, NormalizedPosition,
        RelativePointerMove, RemoteInputEvent, RemoteInputInjector,
    },
    screen_manager::Screen,
};

use super::screen_layout::screen_monitor;

pub(crate) struct WindowsRemoteInputInjector;

impl WindowsRemoteInputInjector {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl RemoteInputInjector for WindowsRemoteInputInjector {
    fn inject(
        &mut self,
        input: RemoteInputEvent,
        _screen: &Screen,
        _screens: &[Screen],
    ) -> eros::Result<()> {
        match input {
            RemoteInputEvent::AbsolutePointerMove(movement) => {
                move_absolute(movement.position, movement.screen_id)
            }
            RemoteInputEvent::Keyboard(input) => keyboard(input),
            RemoteInputEvent::MouseButton(input) => mouse_button(input),
            RemoteInputEvent::RelativePointerMove(movement) => move_relative(movement),
        }
    }
}

fn move_absolute(
    position: NormalizedPosition,
    screen_id: crate::kernel::screen_manager::ScreenId,
) -> eros::Result<()> {
    let monitor = screen_monitor(screen_id)?;
    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor.0, &mut monitor_info) }
        .ok()
        .with_context(|| {
            format!(
                "Failed to query Windows monitor {} for pointer injection",
                screen_id.get()
            )
        })?;

    let virtual_left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virtual_top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let virtual_width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virtual_height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if virtual_width <= 0 || virtual_height <= 0 {
        eros::bail!(
            "Windows reported invalid virtual desktop dimensions {}x{}",
            virtual_width,
            virtual_height
        );
    }

    let rect = monitor_info.rcMonitor;
    let desktop_position = NormalizedPosition {
        x: normalize_windows_axis(
            position.x,
            rect.left,
            rect.right - rect.left,
            virtual_left,
            virtual_width,
        ),
        y: normalize_windows_axis(
            position.y,
            rect.top,
            rect.bottom - rect.top,
            virtual_top,
            virtual_height,
        ),
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: i32::from(desktop_position.x),
                dy: i32::from(desktop_position.y),
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE
                    | MOUSEEVENTF_ABSOLUTE
                    | MOUSEEVENTF_VIRTUALDESK
                    | MOUSEEVENTF_MOVE_NOCOALESCE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_input(input, "absolute pointer movement")
}

fn move_relative(movement: RelativePointerMove) -> eros::Result<()> {
    send_input(
        mouse_input(
            movement.delta_x,
            movement.delta_y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_MOVE_NOCOALESCE,
        ),
        "relative pointer movement",
    )
}

fn keyboard(input: KeyboardInput) -> eros::Result<()> {
    let released = if input.state == InputState::Released {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    let extended = if windows_key_is_extended(input.key) {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    send_input(
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows_key(input.key),
                    wScan: 0,
                    dwFlags: released | extended,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        "keyboard input",
    )
}

fn windows_key_is_extended(key: KeyboardKey) -> bool {
    matches!(
        key,
        KeyboardKey::DELETE
            | KeyboardKey::INSERT
            | KeyboardKey::HOME
            | KeyboardKey::END
            | KeyboardKey::PAGE_UP
            | KeyboardKey::PAGE_DOWN
            | KeyboardKey::ARROW_UP
            | KeyboardKey::ARROW_DOWN
            | KeyboardKey::ARROW_LEFT
            | KeyboardKey::ARROW_RIGHT
            | KeyboardKey::CONTROL_RIGHT
            | KeyboardKey::ALT_RIGHT
            | KeyboardKey::META_LEFT
            | KeyboardKey::META_RIGHT
            | KeyboardKey::PRINT_SCREEN
            | KeyboardKey::MENU
    )
}

fn mouse_button(input: MouseButtonInput) -> eros::Result<()> {
    let pressed = input.state == InputState::Pressed;
    let (flags, data) = match input.button {
        MouseButton::Left => (
            if pressed {
                MOUSEEVENTF_LEFTDOWN
            } else {
                MOUSEEVENTF_LEFTUP
            },
            0,
        ),
        MouseButton::Right => (
            if pressed {
                MOUSEEVENTF_RIGHTDOWN
            } else {
                MOUSEEVENTF_RIGHTUP
            },
            0,
        ),
        MouseButton::Middle => (
            if pressed {
                MOUSEEVENTF_MIDDLEDOWN
            } else {
                MOUSEEVENTF_MIDDLEUP
            },
            0,
        ),
        MouseButton::Back => (
            if pressed {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            1,
        ),
        MouseButton::Forward => (
            if pressed {
                MOUSEEVENTF_XDOWN
            } else {
                MOUSEEVENTF_XUP
            },
            2,
        ),
    };
    send_input(mouse_input(0, 0, data, flags), "mouse button input")
}

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_input(input: INPUT, operation: &str) -> eros::Result<()> {
    let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    if sent != 1 {
        let error = unsafe { GetLastError() };
        eros::bail!("Windows SendInput failed for {}: {:?}", operation, error);
    }
    Ok(())
}

fn windows_key(key: KeyboardKey) -> VIRTUAL_KEY {
    let code = key.wire_code();
    if (u16::from(b'a')..=u16::from(b'z')).contains(&code) {
        return VIRTUAL_KEY(code - u16::from(b'a') + u16::from(b'A'));
    }
    if (u16::from(b'0')..=u16::from(b'9')).contains(&code) {
        return VIRTUAL_KEY(code);
    }
    if (KeyboardKey::F1.wire_code()..=KeyboardKey::F24.wire_code()).contains(&code) {
        return VIRTUAL_KEY(0x70 + code - KeyboardKey::F1.wire_code());
    }
    VIRTUAL_KEY(match key {
        KeyboardKey::SPACE => 0x20,
        KeyboardKey::MINUS => 0xbd,
        KeyboardKey::EQUAL => 0xbb,
        KeyboardKey::LEFT_BRACKET => 0xdb,
        KeyboardKey::RIGHT_BRACKET => 0xdd,
        KeyboardKey::BACKSLASH => 0xdc,
        KeyboardKey::SEMICOLON => 0xba,
        KeyboardKey::QUOTE => 0xde,
        KeyboardKey::BACKQUOTE => 0xc0,
        KeyboardKey::COMMA => 0xbc,
        KeyboardKey::PERIOD => 0xbe,
        KeyboardKey::SLASH => 0xbf,
        KeyboardKey::ESCAPE => 0x1b,
        KeyboardKey::TAB => 0x09,
        KeyboardKey::ENTER => 0x0d,
        KeyboardKey::BACKSPACE => 0x08,
        KeyboardKey::DELETE => 0x2e,
        KeyboardKey::INSERT => 0x2d,
        KeyboardKey::HOME => 0x24,
        KeyboardKey::END => 0x23,
        KeyboardKey::PAGE_UP => 0x21,
        KeyboardKey::PAGE_DOWN => 0x22,
        KeyboardKey::ARROW_UP => 0x26,
        KeyboardKey::ARROW_DOWN => 0x28,
        KeyboardKey::ARROW_LEFT => 0x25,
        KeyboardKey::ARROW_RIGHT => 0x27,
        KeyboardKey::SHIFT_LEFT => 0xa0,
        KeyboardKey::SHIFT_RIGHT => 0xa1,
        KeyboardKey::CONTROL_LEFT => 0xa2,
        KeyboardKey::CONTROL_RIGHT => 0xa3,
        KeyboardKey::ALT_LEFT => 0xa4,
        KeyboardKey::ALT_RIGHT => 0xa5,
        KeyboardKey::META_LEFT => 0x5b,
        KeyboardKey::META_RIGHT => 0x5c,
        KeyboardKey::CAPS_LOCK => 0x14,
        KeyboardKey::SCROLL_LOCK => 0x91,
        KeyboardKey::PAUSE => 0x13,
        KeyboardKey::PRINT_SCREEN => 0x2c,
        KeyboardKey::MENU => 0x5d,
        _ => unreachable!("validated Rabbit keyboard key"),
    })
}

fn normalize_windows_axis(
    local: u16,
    monitor_offset: i32,
    monitor_length: i32,
    desktop_offset: i32,
    desktop_length: i32,
) -> u16 {
    let monitor_position = f64::from(monitor_offset)
        + f64::from(local) / f64::from(u16::MAX) * f64::from(monitor_length.saturating_sub(1));
    let desktop_position = monitor_position - f64::from(desktop_offset);
    let desktop_max = f64::from(desktop_length.saturating_sub(1).max(1));
    (desktop_position / desktop_max * f64::from(u16::MAX))
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16
}
