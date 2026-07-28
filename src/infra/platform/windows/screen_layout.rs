use eros::Context as _;
use windows::{
    Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{
            DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW,
            GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        },
    },
    core::BOOL,
};

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_manager::{
        Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect, ScreenTransform,
    },
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowsMonitorHandle(pub(crate) HMONITOR);

unsafe impl Send for WindowsMonitorHandle {}
unsafe impl Sync for WindowsMonitorHandle {}

#[derive(Debug, kudi::DepInj)]
#[target(WindowsScreenLayoutManager)]
pub(crate) struct WindowsScreenLayoutManagerState {
    screens: Vec<Screen>,
}

/// Creates the screen-layout manager state selected for Windows.
pub(crate) fn create_screen_layout_manager_state() -> eros::Result<WindowsScreenLayoutManagerState>
{
    WindowsScreenLayoutManagerState::new()
}

impl WindowsScreenLayoutManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            screens: enumerate_screens()?,
        })
    }
}

impl<Deps> ScreenLayoutManager for WindowsScreenLayoutManager<Deps>
where
    Deps: AsRef<WindowsScreenLayoutManagerState> + AsMut<WindowsScreenLayoutManagerState>,
{
    fn refresh(&mut self) -> eros::Result<()> {
        let screens = enumerate_screens()?;
        let state = <Deps as AsMut<WindowsScreenLayoutManagerState>>::as_mut(self.prj_ref_mut());
        state.screens = screens;
        Ok(())
    }

    fn screens(&self) -> &[Screen] {
        &<Deps as AsRef<WindowsScreenLayoutManagerState>>::as_ref(self.prj_ref()).screens
    }

    fn screen(&self, id: &ScreenId) -> Option<&Screen> {
        self.screens().iter().find(|screen| screen.id == *id)
    }

    fn primary_screen(&self) -> eros::Result<&Screen> {
        Ok(self
            .screens()
            .iter()
            .find(|screen| screen.layout.rect.x == 0 && screen.layout.rect.y == 0)
            .or_else(|| self.screens().first())
            .with_context(|| "No Windows displays are available")?)
    }
}

pub(crate) fn screen_monitor(id: ScreenId) -> eros::Result<WindowsMonitorHandle> {
    let monitors = enumerate_monitors()?;
    Ok(monitors
        .get(usize::from(id.get()))
        .copied()
        .map(WindowsMonitorHandle)
        .with_context(|| format!("Windows monitor {} is not available", id.get()))?)
}

fn enumerate_screens() -> eros::Result<Vec<Screen>> {
    let monitors = enumerate_monitors()?;
    let mut screens = Vec::new();
    for (index, monitor) in monitors.into_iter().enumerate() {
        let id = ScreenId::try_from(
            u8::try_from(index).with_context(|| "Windows display index exceeds u8")?,
        )
        .with_context(|| "Windows display index exceeds supported Rabbit screen IDs")?;
        if let Some(screen) = monitor_screen(id, monitor)? {
            screens.push(screen);
        }
    }
    Ok(screens)
}

fn enumerate_monitors() -> eros::Result<Vec<HMONITOR>> {
    let mut monitors = Vec::<HMONITOR>::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            Option::<HDC>::None,
            Option::<*const RECT>::None,
            Some(enum_monitor),
            LPARAM((&mut monitors as *mut Vec<HMONITOR>) as isize),
        )
    };
    if !ok.as_bool() {
        eros::bail!("EnumDisplayMonitors failed");
    }
    Ok(monitors)
}

unsafe extern "system" fn enum_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    true.into()
}

fn monitor_screen(id: ScreenId, monitor: HMONITOR) -> eros::Result<Option<Screen>> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if !ok.as_bool() {
        return Ok(None);
    }

    let device = widestring_from_nul(&info.szDevice);
    let mut mode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let mode_ok = unsafe {
        EnumDisplaySettingsW(
            windows::core::PCWSTR(info.szDevice.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
    };
    let rect = info.monitorInfo.rcMonitor;
    let width = u32::try_from(rect.right.saturating_sub(rect.left))
        .with_context(|| "Windows monitor width is negative")?;
    let height = u32::try_from(rect.bottom.saturating_sub(rect.top))
        .with_context(|| "Windows monitor height is negative")?;
    if width == 0 || height == 0 {
        return Ok(None);
    }

    let resolution = if mode_ok.as_bool() && mode.dmPelsWidth > 0 && mode.dmPelsHeight > 0 {
        PixelSize {
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
        }
    } else {
        PixelSize { width, height }
    };
    let frame_rate = if mode_ok.as_bool() && mode.dmDisplayFrequency > 0 {
        FrameRate::new(mode.dmDisplayFrequency, 1)
    } else {
        FrameRate::new(60, 1)
    }
    .with_context(|| "Windows display frame rate is invalid")?;

    Ok(Some(Screen {
        id,
        name: if device.is_empty() {
            format!("Display {}", id.get() + 1)
        } else {
            device
        },
        resolution,
        frame_rate,
        layout: ScreenLayout {
            rect: ScreenRect {
                x: u32::try_from(rect.left.max(0)).unwrap_or(0),
                y: u32::try_from(rect.top.max(0)).unwrap_or(0),
                width,
                height,
            },
            scale: 1.0,
            transform: ScreenTransform::Normal,
        },
    }))
}

fn widestring_from_nul(value: &[u16]) -> String {
    let end = value.iter().position(|c| *c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}
