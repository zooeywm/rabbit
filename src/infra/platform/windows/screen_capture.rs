use std::{cell::RefCell, time::Duration};

use eros::Context as _;
use tracing::{debug, trace};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Texture2D,
            },
            Dxgi::IDXGIDevice,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
            RoGetActivationFactory,
        },
    },
    core::{HSTRING, Interface as _, Ref},
};

use crate::kernel::{
    screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
    screen_manager::{ScreenId, ScreenLayoutManager},
};

use super::screen_layout::{WindowsMonitorHandle, screen_monitor};

#[derive(Debug, kudi::DepInj)]
#[target(WgcScreenCaptureManager)]
pub(crate) struct WgcScreenCaptureManagerState {
    enable_probing: bool,
    probe_interval: Duration,
    d3d: RefCell<Option<WgcD3dDevice>>,
}

impl WgcScreenCaptureManagerState {
    pub(crate) fn new(enable_probing: bool, probe_interval: Duration) -> Self {
        Self {
            enable_probing,
            probe_interval,
            d3d: RefCell::new(None),
        }
    }

    pub(crate) fn d3d(&self) -> eros::Result<WgcD3dDevice> {
        if self.d3d.borrow().is_none() {
            *self.d3d.borrow_mut() = Some(WgcD3dDevice::new()?);
        }
        Ok(self
            .d3d
            .borrow()
            .as_ref()
            .cloned()
            .with_context(|| "Windows D3D11 device was not initialized")?)
    }

    pub(crate) fn probing_enabled(&self) -> bool {
        self.enable_probing
    }

    pub(crate) fn probe_interval(&self) -> Duration {
        self.probe_interval
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WgcD3dDevice {
    direct3d: IDirect3DDevice,
}

impl WgcD3dDevice {
    fn new() -> eros::Result<Self> {
        let mut d3d = None;
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )
        }
        .with_context(|| "Failed to create a D3D11 device for Windows Graphics Capture")?;
        let d3d = d3d.with_context(|| "D3D11CreateDevice returned no ID3D11Device")?;
        let dxgi: IDXGIDevice = d3d
            .cast()
            .with_context(|| "Failed to cast D3D11 device to IDXGIDevice")?;
        let direct3d: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .and_then(|device| device.cast())
            .with_context(|| "Failed to create WinRT Direct3D device from D3D11 device")?;
        Ok(Self { direct3d })
    }
}

#[derive(Debug)]
pub(crate) struct WgcCapturedFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) content_size: crate::kernel::geometry::PixelSize,
    pub(crate) frame_rate: crate::kernel::geometry::FrameRate,
}

pub(crate) struct WgcCaptureLease {
    _session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    _frame_arrived: i64,
}

impl Drop for WgcCaptureLease {
    fn drop(&mut self) {
        let _ = self.frame_pool.RemoveFrameArrived(self._frame_arrived);
        let _ = self.frame_pool.Close();
        let _ = self._session.Close();
    }
}

pub(crate) type WgcFrameReceiver = flume::Receiver<eros::Result<WgcCapturedFrame>>;

impl<Deps> ScreenCaptureManager for WgcScreenCaptureManager<Deps>
where
    Deps: AsRef<WgcScreenCaptureManagerState> + ScreenLayoutManager,
{
    type Lease = WgcCaptureLease;
    type Receiver = WgcFrameReceiver;

    fn acquire(
        &mut self,
        screen_id: &ScreenId,
    ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>> {
        let screen = self
            .prj_ref()
            .screen(screen_id)
            .cloned()
            .with_context(|| format!("Windows screen {} is not available", screen_id.get()))?;
        let monitor = screen_monitor(*screen_id)?;
        let state = <Deps as AsRef<WgcScreenCaptureManagerState>>::as_ref(self.prj_ref());
        let _ = (state.probing_enabled(), state.probe_interval());
        let d3d = state.d3d()?;
        let item = create_capture_item_for_monitor(monitor)
            .with_context(|| format!("Failed to create WGC item for screen {}", screen_id.get()))?;
        let size = item
            .Size()
            .with_context(|| "Failed to query WGC capture item size")?;
        debug!(
            event = "wgc_capture_starting",
            screen_id = screen_id.0,
            width = size.Width,
            height = size.Height,
            "Starting WGC capture"
        );
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d.direct3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .with_context(|| "Failed to create WGC free-threaded frame pool")?;
        let session = frame_pool
            .CreateCaptureSession(&item)
            .with_context(|| "Failed to create WGC capture session")?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);

        let (sender, receiver) = flume::bounded(1);
        let stale = receiver.clone();
        let frame_rate = screen.frame_rate;
        let id = *screen_id;
        let token = frame_pool
            .FrameArrived(&TypedEventHandler::new({
                move |pool: Ref<Direct3D11CaptureFramePool>, _| {
                    if let Some(pool) = pool.as_ref() {
                        match pool
                            .TryGetNextFrame()
                            .and_then(|frame| capture_frame_texture(id, frame_rate, frame))
                        {
                            Ok(frame) => replace_latest(&sender, &stale, Ok(frame)),
                            Err(error) => replace_latest(&sender, &stale, Err(error.into())),
                        }
                    }
                    Ok(())
                }
            }))
            .with_context(|| "Failed to subscribe to WGC frame arrivals")?;
        session
            .StartCapture()
            .with_context(|| "Failed to start WGC capture session")?;
        debug!(
            event = "wgc_capture_started",
            screen_id = screen_id.0,
            "WGC capture started"
        );

        Ok(ScreenCaptureSource {
            lease: WgcCaptureLease {
                _session: session,
                frame_pool,
                _frame_arrived: token,
            },
            receiver,
        })
    }
}

fn replace_latest<T>(sender: &flume::Sender<T>, stale: &flume::Receiver<T>, mut value: T) {
    loop {
        match sender.try_send(value) {
            Ok(()) => return,
            Err(flume::TrySendError::Full(returned)) => {
                value = returned;
                let _ = stale.try_recv();
            }
            Err(flume::TrySendError::Disconnected(_)) => return,
        }
    }
}

fn capture_frame_texture(
    screen_id: ScreenId,
    frame_rate: crate::kernel::geometry::FrameRate,
    frame: Direct3D11CaptureFrame,
) -> windows::core::Result<WgcCapturedFrame> {
    let content = frame.ContentSize()?;
    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
    trace!(
        event = "wgc_frame_arrived",
        screen_id = screen_id.0,
        width = content.Width,
        height = content.Height,
        "WGC frame arrived"
    );
    let _ = frame.Close();
    Ok(WgcCapturedFrame {
        screen_id,
        texture,
        content_size: crate::kernel::geometry::PixelSize {
            width: content.Width.max(0) as u32,
            height: content.Height.max(0) as u32,
        },
        frame_rate,
    })
}

fn create_capture_item_for_monitor(
    monitor: WindowsMonitorHandle,
) -> eros::Result<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop = unsafe {
        RoGetActivationFactory(&HSTRING::from(
            "Windows.Graphics.Capture.GraphicsCaptureItem",
        ))
    }
    .with_context(|| "Failed to get GraphicsCaptureItem interop factory")?;
    Ok(unsafe { interop.CreateForMonitor(monitor.0) }
        .with_context(|| "Failed to create GraphicsCaptureItem for monitor")?)
}
