use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    os::fd::AsRawFd as _,
    os::unix::fs::OpenOptionsExt as _,
};

use eros::Context as _;
use tracing::{debug, info};

use crate::kernel::{
    absolute_pointer::{AbsolutePointerInjector, AbsolutePointerMove, map_screen_to_desktop},
    screen_manager::Screen,
};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const BTN_LEFT: i32 = 0x110;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
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
const UI_SET_PROPBIT: libc::c_ulong = ioctl_write(UINPUT_IOCTL_BASE, 110, size_of::<libc::c_int>());

/// Persistent Linux uinput absolute pointer.
pub(crate) struct LinuxAbsolutePointerInjector {
    device: Option<UInputDevice>,
}

impl LinuxAbsolutePointerInjector {
    pub(crate) fn new() -> Self {
        match UInputDevice::create() {
            Ok(device) => {
                info!(
                    event = "linux_absolute_pointer_ready",
                    "Created Linux uinput absolute pointer"
                );
                Self {
                    device: Some(device),
                }
            }
            Err(error) => {
                debug!(
                    event = "linux_absolute_pointer_unavailable",
                    error = ?error,
                    "Absolute pointer input is not prepared; it will retry on first input"
                );
                Self { device: None }
            }
        }
    }
}

impl AbsolutePointerInjector for LinuxAbsolutePointerInjector {
    fn move_absolute(
        &mut self,
        movement: AbsolutePointerMove,
        screen: &Screen,
        screens: &[Screen],
    ) -> eros::Result<()> {
        if self.device.is_none() {
            self.device = Some(UInputDevice::create().with_context(|| {
                "Failed to open /dev/uinput for absolute pointer injection; configure udev permissions"
            })?);
        }
        let position = map_screen_to_desktop(movement.position, screen, screens);
        self.device
            .as_mut()
            .expect("uinput device was initialized")
            .move_absolute(position.x, position.y)
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
        ioctl_value(fd, UI_SET_KEYBIT, BTN_LEFT)
            .with_context(|| "Failed to enable the uinput mouse button")?;
        ioctl_value(fd, UI_SET_EVBIT, i32::from(EV_ABS))
            .with_context(|| "Failed to enable uinput absolute events")?;
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
            .zip(b"Rabbit absolute pointer".iter().copied())
        {
            *destination = source as libc::c_char;
        }
        ioctl_pointer(fd, UI_DEV_SETUP, &setup)
            .with_context(|| "Failed to configure the uinput absolute pointer")?;
        ioctl_empty(fd, UI_DEV_CREATE)
            .with_context(|| "Failed to create the uinput absolute pointer")?;

        Ok(Self { file })
    }

    fn move_absolute(&mut self, x: u16, y: u16) -> eros::Result<()> {
        self.write_event(EV_ABS, ABS_X, i32::from(x))?;
        self.write_event(EV_ABS, ABS_Y, i32::from(y))?;
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
            .with_context(|| "Failed to write an absolute pointer uinput event")?)
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
