use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use eros::Context as _;
use tracing::{debug, info, trace, warn};
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
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device, ID3D11Texture2D,
            },
            Dxgi::{
                Common::DXGI_FORMAT_B8G8R8A8_UNORM, CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND,
                DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIAdapter1,
                IDXGIDevice, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5,
                IDXGIOutputDuplication,
            },
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

use super::{
    host_video_probe::HostVideoFrameProbe,
    screen_layout::{WindowsMonitorHandle, screen_monitor},
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum WindowsCaptureBackend {
    DesktopDuplication,
    Wgc,
}

#[derive(Debug, kudi::DepInj)]
#[target(WindowsScreenCaptureManager)]
pub(crate) struct WindowsScreenCaptureManagerState {
    backend: WindowsCaptureBackend,
    enable_probing: bool,
    probe_interval: Duration,
    wgc_d3d: RefCell<Option<WgcD3dDevice>>,
}

impl WindowsScreenCaptureManagerState {
    pub(crate) fn new(
        backend: WindowsCaptureBackend,
        enable_probing: bool,
        probe_interval: Duration,
    ) -> Self {
        Self {
            backend,
            enable_probing,
            probe_interval,
            wgc_d3d: RefCell::new(None),
        }
    }

    fn wgc_d3d(&self) -> eros::Result<WgcD3dDevice> {
        if self.wgc_d3d.borrow().is_none() {
            *self.wgc_d3d.borrow_mut() = Some(WgcD3dDevice::new()?);
        }
        Ok(self
            .wgc_d3d
            .borrow()
            .as_ref()
            .cloned()
            .with_context(|| "Windows D3D11 device was not initialized")?)
    }

    fn probing_enabled(&self) -> bool {
        self.enable_probing
    }

    fn probe_interval(&self) -> Duration {
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
pub(crate) struct WindowsCapturedFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) surface: WindowsCapturedSurface,
    pub(crate) content_size: crate::kernel::geometry::PixelSize,
    pub(crate) frame_rate: crate::kernel::geometry::FrameRate,
    pub(crate) probe: Option<HostVideoFrameProbe>,
}

#[derive(Debug)]
pub(crate) struct WindowsCapturedSurface {
    texture: ID3D11Texture2D,
    owner: WindowsCapturedSurfaceOwner,
}

#[derive(Debug)]
enum WindowsCapturedSurfaceOwner {
    Wgc(Direct3D11CaptureFrame),
    DesktopDuplication(flume::Sender<()>),
}

impl WindowsCapturedSurface {
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }
}

impl Drop for WindowsCapturedSurface {
    fn drop(&mut self) {
        match &self.owner {
            WindowsCapturedSurfaceOwner::Wgc(frame) => {
                let _ = frame.Close();
            }
            WindowsCapturedSurfaceOwner::DesktopDuplication(release) => {
                let _ = release.try_send(());
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct WindowsCaptureLease {
    _inner: WindowsCaptureLeaseInner,
}

#[derive(Debug)]
enum WindowsCaptureLeaseInner {
    Wgc {
        _lease: WgcCaptureLease,
    },
    DesktopDuplication {
        _lease: DesktopDuplicationCaptureLease,
    },
}

#[derive(Debug)]
struct WgcCaptureLease {
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

#[derive(Debug)]
struct DesktopDuplicationCaptureLease {
    commands: flume::Sender<DesktopDuplicationCommand>,
    _thread: JoinHandle<()>,
}

impl Drop for DesktopDuplicationCaptureLease {
    fn drop(&mut self) {
        let _ = self.commands.try_send(DesktopDuplicationCommand::Shutdown);
    }
}

#[derive(Debug, Clone, Copy)]
enum DesktopDuplicationCommand {
    Shutdown,
}

pub(crate) type WindowsFrameReceiver = flume::Receiver<eros::Result<WindowsCapturedFrame>>;

impl<Deps> ScreenCaptureManager for WindowsScreenCaptureManager<Deps>
where
    Deps: AsRef<WindowsScreenCaptureManagerState> + ScreenLayoutManager,
{
    type Lease = WindowsCaptureLease;
    type Receiver = WindowsFrameReceiver;

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
        let state = <Deps as AsRef<WindowsScreenCaptureManagerState>>::as_ref(self.prj_ref());
        let probe_interval = state.probe_interval();
        let probe_frame_id = state.probing_enabled().then(|| Arc::new(AtomicU64::new(0)));
        match state.backend {
            WindowsCaptureBackend::DesktopDuplication => acquire_desktop_duplication(
                *screen_id,
                monitor,
                screen.frame_rate,
                probe_frame_id,
                probe_interval,
            ),
            WindowsCaptureBackend::Wgc => acquire_wgc(
                *screen_id,
                monitor,
                screen.frame_rate,
                state.wgc_d3d()?,
                probe_frame_id,
                probe_interval,
            ),
        }
    }
}

fn acquire_wgc(
    screen_id: ScreenId,
    monitor: WindowsMonitorHandle,
    frame_rate: crate::kernel::geometry::FrameRate,
    d3d: WgcD3dDevice,
    probe_frame_id: Option<Arc<AtomicU64>>,
    probe_interval: Duration,
) -> eros::Result<ScreenCaptureSource<WindowsCaptureLease, WindowsFrameReceiver>> {
    info!(
        event = "windows_wgc_capture_selected",
        screen_id = screen_id.get(),
        "Selected Windows Graphics Capture"
    );
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
    let id = screen_id;
    let token = frame_pool
        .FrameArrived(&TypedEventHandler::new({
            let probe_frame_id = probe_frame_id.clone();
            move |pool: Ref<Direct3D11CaptureFramePool>, _| {
                if let Some(pool) = pool.as_ref() {
                    let probe = probe_frame_id.as_ref().map(|next_frame_id| {
                        HostVideoFrameProbe::new(
                            next_frame_id.fetch_add(1, Ordering::Relaxed),
                            probe_interval,
                        )
                    });
                    match pool
                        .TryGetNextFrame()
                        .and_then(|frame| capture_frame_texture(id, frame_rate, frame, probe))
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
        lease: WindowsCaptureLease {
            _inner: WindowsCaptureLeaseInner::Wgc {
                _lease: WgcCaptureLease {
                    _session: session,
                    frame_pool,
                    _frame_arrived: token,
                },
            },
        },
        receiver,
    })
}

fn acquire_desktop_duplication(
    screen_id: ScreenId,
    monitor: WindowsMonitorHandle,
    frame_rate: crate::kernel::geometry::FrameRate,
    probe_frame_id: Option<Arc<AtomicU64>>,
    probe_interval: Duration,
) -> eros::Result<ScreenCaptureSource<WindowsCaptureLease, WindowsFrameReceiver>> {
    let capture = DesktopDuplicationCapture::new(monitor)?;
    let (commands, command_receiver) = flume::bounded(1);
    let (sender, receiver) = flume::bounded(1);
    let thread = thread::Builder::new()
        .name(format!("rabbit-ddx-{}", screen_id.get()))
        .spawn(move || {
            if let Err(error) = run_desktop_duplication_capture(
                capture,
                screen_id,
                frame_rate,
                probe_frame_id,
                probe_interval,
                &command_receiver,
                &sender,
            ) {
                let _ = sender.try_send(Err(error));
            }
        })
        .with_context(|| "Failed to start Windows Desktop Duplication capture thread")?;

    Ok(ScreenCaptureSource {
        lease: WindowsCaptureLease {
            _inner: WindowsCaptureLeaseInner::DesktopDuplication {
                _lease: DesktopDuplicationCaptureLease {
                    commands,
                    _thread: thread,
                },
            },
        },
        receiver,
    })
}

struct DesktopDuplicationCapture {
    _device: ID3D11Device,
    duplication: IDXGIOutputDuplication,
    size: crate::kernel::geometry::PixelSize,
}

impl DesktopDuplicationCapture {
    fn new(monitor: WindowsMonitorHandle) -> eros::Result<Self> {
        let (adapter, output) = find_dxgi_output(monitor)?;
        let device = create_d3d_device_for_adapter(&adapter)?;
        let duplication = create_output_duplication(&output, &device)?;
        let description = unsafe { duplication.GetDesc() };
        let size = crate::kernel::geometry::PixelSize {
            width: description.ModeDesc.Width,
            height: description.ModeDesc.Height,
        };
        if size.width == 0 || size.height == 0 {
            eros::bail!("Desktop Duplication returned an empty desktop mode");
        }
        info!(
            event = "windows_desktop_duplication_selected",
            width = size.width,
            height = size.height,
            format = ?description.ModeDesc.Format,
            "Selected Windows Desktop Duplication capture"
        );
        Ok(Self {
            _device: device,
            duplication,
            size,
        })
    }
}

fn run_desktop_duplication_capture(
    capture: DesktopDuplicationCapture,
    screen_id: ScreenId,
    frame_rate: crate::kernel::geometry::FrameRate,
    probe_frame_id: Option<Arc<AtomicU64>>,
    probe_interval: Duration,
    commands: &flume::Receiver<DesktopDuplicationCommand>,
    frames: &flume::Sender<eros::Result<WindowsCapturedFrame>>,
) -> eros::Result<()> {
    loop {
        if matches!(
            commands.try_recv(),
            Ok(DesktopDuplicationCommand::Shutdown) | Err(flume::TryRecvError::Disconnected)
        ) {
            return Ok(());
        }

        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match unsafe {
            capture
                .duplication
                .AcquireNextFrame(16, &mut frame_info, &mut resource)
        } {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(error) => {
                Err(error).with_context(|| "Failed to acquire a Desktop Duplication frame")?;
                unreachable!()
            }
        }
        if frame_info.LastPresentTime == 0 {
            unsafe { capture.duplication.ReleaseFrame() }
                .with_context(|| "Failed to release a pointer-only Desktop Duplication frame")?;
            continue;
        }
        let texture = (|| -> eros::Result<ID3D11Texture2D> {
            let resource = resource.with_context(
                || "Desktop Duplication acquired a frame without a desktop resource",
            )?;
            Ok(resource
                .cast()
                .with_context(|| "Desktop Duplication resource is not a D3D11 texture")?)
        })();
        let texture = match texture {
            Ok(texture) => texture,
            Err(error) => {
                let _ = unsafe { capture.duplication.ReleaseFrame() };
                return Err(error);
            }
        };
        let (release, released) = flume::bounded(1);
        let probe = probe_frame_id.as_ref().map(|next_frame_id| {
            HostVideoFrameProbe::new(
                next_frame_id.fetch_add(1, Ordering::Relaxed),
                probe_interval,
            )
        });
        let frame = WindowsCapturedFrame {
            screen_id,
            surface: WindowsCapturedSurface {
                texture,
                owner: WindowsCapturedSurfaceOwner::DesktopDuplication(release),
            },
            content_size: capture.size,
            frame_rate,
            probe,
        };
        if frames.send(Ok(frame)).is_err() {
            let _ = released.recv();
            let _ = unsafe { capture.duplication.ReleaseFrame() };
            return Ok(());
        }

        let _ = released.recv();
        unsafe { capture.duplication.ReleaseFrame() }
            .with_context(|| "Failed to release a Desktop Duplication frame")?;
    }
}

fn find_dxgi_output(monitor: WindowsMonitorHandle) -> eros::Result<(IDXGIAdapter1, IDXGIOutput)> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.with_context(|| "Failed to create DXGI factory")?;
    for adapter_index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => {
                Err(error).with_context(|| "Failed to enumerate DXGI adapters")?;
                unreachable!()
            }
        };
        for output_index in 0.. {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => {
                    Err(error).with_context(|| "Failed to enumerate DXGI outputs")?;
                    unreachable!()
                }
            };
            let description = unsafe { output.GetDesc() }
                .with_context(|| "Failed to query DXGI output description")?;
            if description.Monitor == monitor.0 {
                return Ok((adapter, output));
            }
        }
    }
    eros::bail!("No DXGI output matches the selected Windows monitor")
}

fn create_d3d_device_for_adapter(adapter: &IDXGIAdapter1) -> eros::Result<ID3D11Device> {
    let adapter: IDXGIAdapter = adapter
        .cast()
        .with_context(|| "Failed to access selected DXGI adapter")?;
    let mut device = None;
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .with_context(|| "Failed to create D3D11 device on the capture adapter")?;
    Ok(device.with_context(|| "D3D11CreateDevice returned no capture device")?)
}

fn create_output_duplication(
    output: &IDXGIOutput,
    device: &ID3D11Device,
) -> eros::Result<IDXGIOutputDuplication> {
    if let Ok(output5) = output.cast::<IDXGIOutput5>() {
        return Ok(
            unsafe { output5.DuplicateOutput1(device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) }
                .with_context(|| "IDXGIOutput5::DuplicateOutput1 failed")?,
        );
    }

    warn!(
        event = "windows_desktop_duplication_legacy",
        "IDXGIOutput5 is unavailable; using legacy DuplicateOutput"
    );
    let output1: IDXGIOutput1 = output
        .cast()
        .with_context(|| "DXGI output does not support Desktop Duplication")?;
    Ok(unsafe { output1.DuplicateOutput(device) }
        .with_context(|| "IDXGIOutput1::DuplicateOutput failed")?)
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
    probe: Option<HostVideoFrameProbe>,
) -> windows::core::Result<WindowsCapturedFrame> {
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
    Ok(WindowsCapturedFrame {
        screen_id,
        surface: WindowsCapturedSurface {
            texture,
            owner: WindowsCapturedSurfaceOwner::Wgc(frame),
        },
        content_size: crate::kernel::geometry::PixelSize {
            width: content.Width.max(0) as u32,
            height: content.Height.max(0) as u32,
        },
        frame_rate,
        probe,
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
