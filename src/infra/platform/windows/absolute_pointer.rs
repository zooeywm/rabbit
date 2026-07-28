use eros::Context as _;
use windows::Win32::{
    Foundation::GetLastError,
    Graphics::Gdi::{GetMonitorInfoW, MONITORINFO},
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_MOVE_NOCOALESCE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        },
    },
};

use crate::kernel::{
    absolute_pointer::{AbsolutePointerInjector, AbsolutePointerMove, NormalizedPosition},
    screen_manager::Screen,
};

use super::screen_layout::screen_monitor;

pub(crate) struct WindowsAbsolutePointerInjector;

impl WindowsAbsolutePointerInjector {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl AbsolutePointerInjector for WindowsAbsolutePointerInjector {
    fn move_absolute(
        &mut self,
        movement: AbsolutePointerMove,
        _screen: &Screen,
        _screens: &[Screen],
    ) -> eros::Result<()> {
        let monitor = screen_monitor(movement.screen_id)?;
        let mut monitor_info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(monitor.0, &mut monitor_info) }
            .ok()
            .with_context(|| {
                format!(
                    "Failed to query Windows monitor {} for pointer injection",
                    movement.screen_id.get()
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
                movement.position.x,
                rect.left,
                rect.right - rect.left,
                virtual_left,
                virtual_width,
            ),
            y: normalize_windows_axis(
                movement.position.y,
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
        let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
        if sent != 1 {
            let error = unsafe { GetLastError() };
            eros::bail!(
                "Windows SendInput failed for absolute pointer movement: {:?}",
                error
            );
        }
        Ok(())
    }
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
