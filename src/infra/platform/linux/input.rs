use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    os::fd::AsRawFd as _,
    os::unix::fs::OpenOptionsExt as _,
};

use eros::Context as _;
use tracing::{debug, info};

use crate::kernel::{
    input::{
        InputState, KeyboardInput, KeyboardKey, MouseButton, MouseButtonInput, RelativePointerMove,
        RemoteInputEvent, RemoteInputInjector, map_screen_to_desktop,
    },
    screen_manager::Screen,
};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const INPUT_PROP_POINTER: i32 = 0x00;
const BUS_USB: u16 = 0x03;
const UINPUT_IOCTL_BASE: u64 = b'U' as u64;
const UI_DEV_CREATE: libc::c_ulong = ioctl_none(UINPUT_IOCTL_BASE, 1);
const UI_DEV_DESTROY: libc::c_ulong = ioctl_none(UINPUT_IOCTL_BASE, 2);
const UI_DEV_SETUP: libc::c_ulong =
    ioctl_write(UINPUT_IOCTL_BASE, 3, size_of::<libc::uinput_setup>());
const UI_ABS_SETUP: libc::c_ulong =
    ioctl_write(UINPUT_IOCTL_BASE, 4, size_of::<libc::uinput_abs_setup>());
const UI_SET_EVBIT: libc::c_ulong = ioctl_write(UINPUT_IOCTL_BASE, 100, size_of::<libc::c_int>());
const UI_SET_KEYBIT: libc::c_ulong = ioctl_write(UINPUT_IOCTL_BASE, 101, size_of::<libc::c_int>());
const UI_SET_RELBIT: libc::c_ulong = ioctl_write(UINPUT_IOCTL_BASE, 102, size_of::<libc::c_int>());
const UI_SET_PROPBIT: libc::c_ulong = ioctl_write(UINPUT_IOCTL_BASE, 110, size_of::<libc::c_int>());

/// Persistent Linux uinput keyboard and pointer injector.
pub(crate) struct LinuxRemoteInputInjector {
    device: Option<UInputDevice>,
}

impl LinuxRemoteInputInjector {
    pub(crate) fn new() -> Self {
        match UInputDevice::create() {
            Ok(device) => {
                info!(
                    event = "linux_remote_input_ready",
                    "Created Linux uinput remote input device"
                );
                Self {
                    device: Some(device),
                }
            }
            Err(error) => {
                debug!(
                    event = "linux_remote_input_unavailable",
                    error = ?error,
                    "Remote input is not prepared; it will retry on first input"
                );
                Self { device: None }
            }
        }
    }
}

impl RemoteInputInjector for LinuxRemoteInputInjector {
    fn inject(
        &mut self,
        input: RemoteInputEvent,
        screen: &Screen,
        screens: &[Screen],
    ) -> eros::Result<()> {
        if self.device.is_none() {
            self.device = Some(UInputDevice::create().with_context(|| {
                "Failed to open /dev/uinput for remote input injection; configure udev permissions"
            })?);
        }
        let device = self.device.as_mut().expect("uinput device was initialized");
        match input {
            RemoteInputEvent::AbsolutePointerMove(movement) => {
                let position = map_screen_to_desktop(movement.position, screen, screens);
                device.move_absolute(position.x, position.y)
            }
            RemoteInputEvent::Keyboard(input) => device.keyboard(input),
            RemoteInputEvent::MouseButton(input) => device.mouse_button(input),
            RemoteInputEvent::RelativePointerMove(movement) => device.move_relative(movement),
        }
    }
}

struct UInputDevice {
    file: File,
}

impl UInputDevice {
    fn create() -> eros::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .with_context(|| "Failed to open /dev/uinput")?;
        let fd = file.as_raw_fd();

        ioctl_value(fd, UI_SET_EVBIT, i32::from(EV_KEY))
            .with_context(|| "Failed to enable uinput key events")?;
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ] {
            ioctl_value(fd, UI_SET_KEYBIT, linux_button_code(button))
                .with_context(|| "Failed to enable a uinput mouse button")?;
        }
        for key in KeyboardKey::ALL {
            ioctl_value(fd, UI_SET_KEYBIT, linux_key_code(key))
                .with_context(|| "Failed to enable a uinput keyboard key")?;
        }
        ioctl_value(fd, UI_SET_EVBIT, i32::from(EV_ABS))
            .with_context(|| "Failed to enable uinput absolute events")?;
        ioctl_value(fd, UI_SET_EVBIT, i32::from(EV_REL))
            .with_context(|| "Failed to enable uinput relative events")?;
        ioctl_value(fd, UI_SET_RELBIT, i32::from(REL_X))
            .with_context(|| "Failed to enable the uinput relative X axis")?;
        ioctl_value(fd, UI_SET_RELBIT, i32::from(REL_Y))
            .with_context(|| "Failed to enable the uinput relative Y axis")?;
        ioctl_value(fd, UI_SET_PROPBIT, INPUT_PROP_POINTER)
            .with_context(|| "Failed to mark uinput device as a pointer")?;
        setup_axis(fd, ABS_X)?;
        setup_axis(fd, ABS_Y)?;

        let mut setup: libc::uinput_setup = unsafe { std::mem::zeroed() };
        setup.id.bustype = BUS_USB;
        setup.id.vendor = 0x5242;
        setup.id.product = 0x0001;
        setup.id.version = 1;
        for (destination, source) in setup
            .name
            .iter_mut()
            .zip(b"Rabbit remote input".iter().copied())
        {
            *destination = source as libc::c_char;
        }
        ioctl_pointer(fd, UI_DEV_SETUP, &setup)
            .with_context(|| "Failed to configure the uinput remote input device")?;
        ioctl_empty(fd, UI_DEV_CREATE)
            .with_context(|| "Failed to create the uinput remote input device")?;

        Ok(Self { file })
    }

    fn move_absolute(&mut self, x: u16, y: u16) -> eros::Result<()> {
        self.write_event(EV_ABS, ABS_X, i32::from(x))?;
        self.write_event(EV_ABS, ABS_Y, i32::from(y))?;
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    fn move_relative(&mut self, movement: RelativePointerMove) -> eros::Result<()> {
        self.write_event(EV_REL, REL_X, movement.delta_x)?;
        self.write_event(EV_REL, REL_Y, movement.delta_y)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    fn keyboard(&mut self, input: KeyboardInput) -> eros::Result<()> {
        let value = match (input.state, input.repeat) {
            (InputState::Pressed, false) => 1,
            (InputState::Pressed, true) => 2,
            (InputState::Released, _) => 0,
        };
        self.write_event(EV_KEY, linux_key_code(input.key) as u16, value)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    fn mouse_button(&mut self, input: MouseButtonInput) -> eros::Result<()> {
        let value = i32::from(input.state == InputState::Pressed);
        self.write_event(EV_KEY, linux_button_code(input.button) as u16, value)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    fn write_event(&mut self, event_type: u16, code: u16, value: i32) -> eros::Result<()> {
        let mut event: libc::input_event = unsafe { std::mem::zeroed() };
        event.type_ = event_type;
        event.code = code;
        event.value = value;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const libc::input_event).cast::<u8>(),
                size_of::<libc::input_event>(),
            )
        };
        Ok(self
            .file
            .write_all(bytes)
            .with_context(|| "Failed to write a remote uinput event")?)
    }
}

fn linux_button_code(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::Back => 0x116,
        MouseButton::Forward => 0x115,
    }
}

fn linux_key_code(key: KeyboardKey) -> i32 {
    match key {
        KeyboardKey::A => 30,
        KeyboardKey::B => 48,
        KeyboardKey::C => 46,
        KeyboardKey::D => 32,
        KeyboardKey::E => 18,
        KeyboardKey::F => 33,
        KeyboardKey::G => 34,
        KeyboardKey::H => 35,
        KeyboardKey::I => 23,
        KeyboardKey::J => 36,
        KeyboardKey::K => 37,
        KeyboardKey::L => 38,
        KeyboardKey::M => 50,
        KeyboardKey::N => 49,
        KeyboardKey::O => 24,
        KeyboardKey::P => 25,
        KeyboardKey::Q => 16,
        KeyboardKey::R => 19,
        KeyboardKey::S => 31,
        KeyboardKey::T => 20,
        KeyboardKey::U => 22,
        KeyboardKey::V => 47,
        KeyboardKey::W => 17,
        KeyboardKey::X => 45,
        KeyboardKey::Y => 21,
        KeyboardKey::Z => 44,
        KeyboardKey::DIGIT_0 => 11,
        KeyboardKey::DIGIT_1 => 2,
        KeyboardKey::DIGIT_2 => 3,
        KeyboardKey::DIGIT_3 => 4,
        KeyboardKey::DIGIT_4 => 5,
        KeyboardKey::DIGIT_5 => 6,
        KeyboardKey::DIGIT_6 => 7,
        KeyboardKey::DIGIT_7 => 8,
        KeyboardKey::DIGIT_8 => 9,
        KeyboardKey::DIGIT_9 => 10,
        KeyboardKey::SPACE => 57,
        KeyboardKey::MINUS => 12,
        KeyboardKey::EQUAL => 13,
        KeyboardKey::LEFT_BRACKET => 26,
        KeyboardKey::RIGHT_BRACKET => 27,
        KeyboardKey::BACKSLASH => 43,
        KeyboardKey::SEMICOLON => 39,
        KeyboardKey::QUOTE => 40,
        KeyboardKey::BACKQUOTE => 41,
        KeyboardKey::COMMA => 51,
        KeyboardKey::PERIOD => 52,
        KeyboardKey::SLASH => 53,
        KeyboardKey::ESCAPE => 1,
        KeyboardKey::TAB => 15,
        KeyboardKey::ENTER => 28,
        KeyboardKey::BACKSPACE => 14,
        KeyboardKey::DELETE => 111,
        KeyboardKey::INSERT => 110,
        KeyboardKey::HOME => 102,
        KeyboardKey::END => 107,
        KeyboardKey::PAGE_UP => 104,
        KeyboardKey::PAGE_DOWN => 109,
        KeyboardKey::ARROW_UP => 103,
        KeyboardKey::ARROW_DOWN => 108,
        KeyboardKey::ARROW_LEFT => 105,
        KeyboardKey::ARROW_RIGHT => 106,
        KeyboardKey::SHIFT_LEFT => 42,
        KeyboardKey::SHIFT_RIGHT => 54,
        KeyboardKey::CONTROL_LEFT => 29,
        KeyboardKey::CONTROL_RIGHT => 97,
        KeyboardKey::ALT_LEFT => 56,
        KeyboardKey::ALT_RIGHT => 100,
        KeyboardKey::META_LEFT => 125,
        KeyboardKey::META_RIGHT => 126,
        KeyboardKey::CAPS_LOCK => 58,
        KeyboardKey::SCROLL_LOCK => 70,
        KeyboardKey::PAUSE => 119,
        KeyboardKey::PRINT_SCREEN => 99,
        KeyboardKey::MENU => 139,
        KeyboardKey::F1 => 59,
        KeyboardKey::F2 => 60,
        KeyboardKey::F3 => 61,
        KeyboardKey::F4 => 62,
        KeyboardKey::F5 => 63,
        KeyboardKey::F6 => 64,
        KeyboardKey::F7 => 65,
        KeyboardKey::F8 => 66,
        KeyboardKey::F9 => 67,
        KeyboardKey::F10 => 68,
        KeyboardKey::F11 => 87,
        KeyboardKey::F12 => 88,
        KeyboardKey::F13 => 183,
        KeyboardKey::F14 => 184,
        KeyboardKey::F15 => 185,
        KeyboardKey::F16 => 186,
        KeyboardKey::F17 => 187,
        KeyboardKey::F18 => 188,
        KeyboardKey::F19 => 189,
        KeyboardKey::F20 => 190,
        KeyboardKey::F21 => 191,
        KeyboardKey::F22 => 192,
        KeyboardKey::F23 => 193,
        KeyboardKey::F24 => 194,
        _ => unreachable!("validated Rabbit keyboard key"),
    }
}

impl Drop for UInputDevice {
    fn drop(&mut self) {
        let _ = ioctl_empty(self.file.as_raw_fd(), UI_DEV_DESTROY);
    }
}

fn setup_axis(fd: libc::c_int, code: u16) -> eros::Result<()> {
    let setup = libc::uinput_abs_setup {
        code,
        absinfo: libc::input_absinfo {
            value: 0,
            minimum: 0,
            maximum: i32::from(u16::MAX),
            fuzz: 0,
            flat: 0,
            resolution: 0,
        },
    };
    Ok(ioctl_pointer(fd, UI_ABS_SETUP, &setup)
        .with_context(|| format!("Failed to configure uinput absolute axis {code}"))?)
}

fn ioctl_value(fd: libc::c_int, request: libc::c_ulong, value: libc::c_int) -> std::io::Result<()> {
    let result = unsafe { libc::ioctl(fd, request, value) };
    ioctl_result(result)
}

fn ioctl_pointer<T>(fd: libc::c_int, request: libc::c_ulong, value: &T) -> std::io::Result<()> {
    let result = unsafe { libc::ioctl(fd, request, value as *const T) };
    ioctl_result(result)
}

fn ioctl_empty(fd: libc::c_int, request: libc::c_ulong) -> std::io::Result<()> {
    let result = unsafe { libc::ioctl(fd, request) };
    ioctl_result(result)
}

fn ioctl_result(result: libc::c_int) -> std::io::Result<()> {
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

const fn ioctl_none(kind: u64, number: u64) -> libc::c_ulong {
    ((kind << 8) | number) as libc::c_ulong
}

const fn ioctl_write(kind: u64, number: u64, size: usize) -> libc::c_ulong {
    ((1u64 << 30) | ((size as u64) << 16) | (kind << 8) | number) as libc::c_ulong
}
