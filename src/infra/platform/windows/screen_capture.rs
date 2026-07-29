use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
        Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_OBJECT_0},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
            Direct3D10::ID3D10Multithread,
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::DXGI_FORMAT_B8G8R8A8_UNORM, CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND,
                DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIAdapter1,
                IDXGIDevice, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5,
                IDXGIOutputDuplication, IDXGIResource,
            },
        },
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE,
            SetWaitableTimerEx, TIMER_ALL_ACCESS, WaitForSingleObject,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
            RoGetActivationFactory,
        },
    },
    core::{HSTRING, Interface as _, PCWSTR, Ref},
};

use crate::kernel::{
    geometry::FrameRate,
    screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
    screen_manager::{ScreenId, ScreenLayoutManager},
    video_encoder::VideoFrameRateMode,
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
    next_frame_schedule: Cell<Option<WindowsCaptureFrameSchedule>>,
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
            next_frame_schedule: Cell::new(None),
        }
    }

    pub(crate) fn set_next_frame_schedule(
        &self,
        frame_rate_mode: VideoFrameRateMode,
        frame_rate: FrameRate,
    ) {
        self.next_frame_schedule
            .set(Some(WindowsCaptureFrameSchedule {
                frame_rate_mode,
                frame_rate,
            }));
    }

    fn take_next_frame_schedule(
        &self,
        source_frame_rate: FrameRate,
    ) -> WindowsCaptureFrameSchedule {
        self.next_frame_schedule
            .take()
            .unwrap_or(WindowsCaptureFrameSchedule {
                frame_rate_mode: VideoFrameRateMode::Dynamic,
                frame_rate: source_frame_rate,
            })
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

#[derive(Debug, Clone, Copy)]
struct WindowsCaptureFrameSchedule {
    frame_rate_mode: VideoFrameRateMode,
    frame_rate: FrameRate,
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
    pub(crate) fixed_rate_paced: bool,
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
    thread: Option<JoinHandle<()>>,
}

impl Drop for DesktopDuplicationCaptureLease {
    fn drop(&mut self) {
        let _ = self.commands.try_send(DesktopDuplicationCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(()) => info!(
                    event = "windows_desktop_duplication_released",
                    "Released Windows Desktop Duplication before returning from stream shutdown"
                ),
                Err(payload) => warn!(
                    event = "windows_desktop_duplication_thread_panicked",
                    panic = ?payload,
                    "Windows Desktop Duplication capture thread panicked during shutdown"
                ),
            }
        }
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
        let frame_schedule = state.take_next_frame_schedule(screen.frame_rate);
        match state.backend {
            WindowsCaptureBackend::DesktopDuplication => acquire_desktop_duplication(
                *screen_id,
                monitor,
                screen.frame_rate,
                frame_schedule,
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
                        Ok(frame) => replace_latest(sender.clone(), stale.clone(), Ok(frame)),
                        Err(error) => {
                            replace_latest(sender.clone(), stale.clone(), Err(error.into()))
                        }
                    }
                }
                Ok(())
            }
        }))
        .with_context(|| "Failed to subscribe to WGC frame arrivals")?;
    session
        .StartCapture()
        .with_context(|| "Failed to start WGC capture session")?;
    info!(
        event = "windows_wgc_capture_selected",
        screen_id = screen_id.get(),
        backend = "windows-graphics-capture",
        graphics_api = "d3d11",
        pixel_format = "bgra8-unorm",
        memory = "d3d11-texture",
        update_source = "frame-arrived",
        width = size.Width,
        height = size.Height,
        "Selected Windows screen capture pipeline"
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
    frame_schedule: WindowsCaptureFrameSchedule,
    probe_frame_id: Option<Arc<AtomicU64>>,
    probe_interval: Duration,
) -> eros::Result<ScreenCaptureSource<WindowsCaptureLease, WindowsFrameReceiver>> {
    let capture = DesktopDuplicationCapture::new(monitor)?;
    let (commands, command_receiver) = flume::bounded(1);
    let (sender, receiver) = flume::bounded(1);
    let thread = thread::Builder::new()
        .name(format!("rabbit-ddx-{}", screen_id.get()))
        .spawn(move || {
            let output = DesktopDuplicationCaptureOutput {
                screen_id,
                probe_frame_id,
                probe_interval,
                commands: command_receiver,
                frames: sender.clone(),
            };
            if let Err(error) =
                run_desktop_duplication_capture(capture, frame_rate, frame_schedule, &output)
            {
                let _ = sender.try_send(Err(error));
            }
        })
        .with_context(|| "Failed to start Windows Desktop Duplication capture thread")?;

    Ok(ScreenCaptureSource {
        lease: WindowsCaptureLease {
            _inner: WindowsCaptureLeaseInner::DesktopDuplication {
                _lease: DesktopDuplicationCaptureLease {
                    commands,
                    thread: Some(thread),
                },
            },
        },
        receiver,
    })
}

struct DesktopDuplicationCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    size: crate::kernel::geometry::PixelSize,
    texture: Option<ID3D11Texture2D>,
}

impl DesktopDuplicationCapture {
    fn new(monitor: WindowsMonitorHandle) -> eros::Result<Self> {
        let (adapter, output) = find_dxgi_output(monitor)?;
        let device = create_d3d_device_for_adapter(&adapter)?;
        let context = unsafe { device.GetImmediateContext() }
            .with_context(|| "Failed to get Desktop Duplication D3D11 context")?;
        let multithread: ID3D10Multithread = context
            .cast()
            .with_context(|| "Desktop Duplication context has no multithread protection")?;
        let _ = unsafe { multithread.SetMultithreadProtected(true) };
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
            backend = "dxgi-desktop-duplication",
            graphics_api = "d3d11",
            memory = "d3d11-texture",
            update_source = "acquire-next-frame",
            width = size.width,
            height = size.height,
            format = ?description.ModeDesc.Format,
            "Selected Windows screen capture pipeline"
        );
        Ok(Self {
            device,
            context,
            duplication,
            size,
            texture: None,
        })
    }

    fn acquire_latest(&mut self, timeout_ms: u32) -> eros::Result<bool> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match unsafe {
            self.duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        } {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
            Err(error) => {
                Err(error).with_context(|| "Failed to acquire a Desktop Duplication frame")?;
                unreachable!()
            }
        }

        let result = if frame_info.LastPresentTime == 0 {
            Ok(false)
        } else {
            self.copy_desktop_texture(resource).map(|()| true)
        };
        let release = unsafe { self.duplication.ReleaseFrame() }
            .with_context(|| "Failed to release a Desktop Duplication frame");
        result?;
        release?;
        Ok(frame_info.LastPresentTime != 0)
    }

    fn copy_desktop_texture(&mut self, resource: Option<IDXGIResource>) -> eros::Result<()> {
        let resource =
            resource.with_context(|| "Desktop Duplication returned no desktop resource")?;
        let source: ID3D11Texture2D = resource
            .cast()
            .with_context(|| "Desktop Duplication resource is not a D3D11 texture")?;
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut source_desc) };
        if source_desc.Width != self.size.width
            || source_desc.Height != self.size.height
            || source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        {
            eros::bail!(
                "Desktop Duplication texture changed to {}x{} {:?}",
                source_desc.Width,
                source_desc.Height,
                source_desc.Format
            );
        }
        if self.texture.is_none() {
            let owned_desc = D3D11_TEXTURE2D_DESC {
                Width: source_desc.Width,
                Height: source_desc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: source_desc.Format,
                SampleDesc: source_desc.SampleDesc,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture = None;
            unsafe {
                self.device
                    .CreateTexture2D(&owned_desc, None, Some(&mut texture))
            }
            .with_context(|| "Failed to allocate Desktop Duplication snapshot texture")?;
            self.texture = Some(texture.with_context(
                || "CreateTexture2D returned no Desktop Duplication snapshot texture",
            )?);
        }
        let texture = self
            .texture
            .as_ref()
            .with_context(|| "Desktop Duplication snapshot texture is unavailable")?;
        unsafe { self.context.CopyResource(texture, &source) };
        Ok(())
    }
}

fn run_desktop_duplication_capture(
    mut capture: DesktopDuplicationCapture,
    source_frame_rate: crate::kernel::geometry::FrameRate,
    frame_schedule: WindowsCaptureFrameSchedule,
    output: &DesktopDuplicationCaptureOutput,
) -> eros::Result<()> {
    match frame_schedule.frame_rate_mode {
        VideoFrameRateMode::Dynamic => {
            run_dynamic_desktop_duplication_capture(&mut capture, source_frame_rate, output)
        }
        VideoFrameRateMode::Fixed => {
            run_fixed_desktop_duplication_capture(&mut capture, frame_schedule.frame_rate, output)
        }
    }
}

fn run_dynamic_desktop_duplication_capture(
    capture: &mut DesktopDuplicationCapture,
    frame_rate: FrameRate,
    output: &DesktopDuplicationCaptureOutput,
) -> eros::Result<()> {
    const SHUTDOWN_POLL_INTERVAL_MS: u32 = 100;
    loop {
        if output.shutdown_requested() {
            return Ok(());
        }

        if !capture.acquire_latest(SHUTDOWN_POLL_INTERVAL_MS)? {
            // AcquireNextFrame holds the D3D11 device's unfair multithread lock
            // for the entire timeout. Yield it while the desktop is idle so an
            // on-demand IDR or encoder teardown is not starved by the capture
            // thread immediately entering another blocking acquire.
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        if !output.publish(capture, frame_rate, false)? {
            return Ok(());
        }
    }
}

fn run_fixed_desktop_duplication_capture(
    capture: &mut DesktopDuplicationCapture,
    frame_rate: FrameRate,
    output: &DesktopDuplicationCaptureOutput,
) -> eros::Result<()> {
    let timer = HighResolutionWaitableTimer::new()?;
    let mut clock = FixedCaptureClock::new(frame_rate);
    info!(
        event = "windows_desktop_duplication_fixed_rate_configured",
        screen_id = output.screen_id.get(),
        frame_rate_numerator = frame_rate.numerator(),
        frame_rate_denominator = frame_rate.denominator(),
        timer = "waitable-timer",
        high_resolution = timer.high_resolution,
        acquire_timeout_ms = 0,
        "Configured fixed-rate Windows screen capture"
    );
    loop {
        if output.shutdown_requested() {
            return Ok(());
        }
        timer.wait(clock.delay())?;
        clock.advance();
        if output.shutdown_requested() {
            return Ok(());
        }

        let _updated = capture.acquire_latest(0)?;
        if capture.texture.is_none() {
            continue;
        }
        if !output.publish(capture, frame_rate, true)? {
            return Ok(());
        }
    }
}

struct DesktopDuplicationCaptureOutput {
    screen_id: ScreenId,
    probe_frame_id: Option<Arc<AtomicU64>>,
    probe_interval: Duration,
    commands: flume::Receiver<DesktopDuplicationCommand>,
    frames: flume::Sender<eros::Result<WindowsCapturedFrame>>,
}

impl DesktopDuplicationCaptureOutput {
    fn publish(
        &self,
        capture: &DesktopDuplicationCapture,
        frame_rate: FrameRate,
        fixed_rate_paced: bool,
    ) -> eros::Result<bool> {
        let texture = capture
            .texture
            .as_ref()
            .cloned()
            .with_context(|| "Desktop Duplication snapshot texture is unavailable")?;
        let (release, released) = flume::bounded(1);
        let probe = self.probe_frame_id.as_ref().map(|next_frame_id| {
            HostVideoFrameProbe::new(
                next_frame_id.fetch_add(1, Ordering::Relaxed),
                self.probe_interval,
            )
        });
        let frame = WindowsCapturedFrame {
            screen_id: self.screen_id,
            surface: WindowsCapturedSurface {
                texture,
                owner: WindowsCapturedSurfaceOwner::DesktopDuplication(release),
            },
            content_size: capture.size,
            frame_rate,
            fixed_rate_paced,
            probe,
        };
        if self.frames.send(Ok(frame)).is_err() {
            let _ = released.recv();
            return Ok(false);
        }

        loop {
            match released.recv_timeout(Duration::from_millis(10)) {
                Ok(()) | Err(flume::RecvTimeoutError::Disconnected) => return Ok(true),
                Err(flume::RecvTimeoutError::Timeout) if self.shutdown_requested() => {
                    return Ok(false);
                }
                Err(flume::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn shutdown_requested(&self) -> bool {
        matches!(
            self.commands.try_recv(),
            Ok(DesktopDuplicationCommand::Shutdown) | Err(flume::TryRecvError::Disconnected)
        )
    }
}

struct HighResolutionWaitableTimer {
    handle: HANDLE,
    high_resolution: bool,
}

impl HighResolutionWaitableTimer {
    fn new() -> eros::Result<Self> {
        match unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        } {
            Ok(handle) => Ok(Self {
                handle,
                high_resolution: true,
            }),
            Err(high_resolution_error) => {
                warn!(
                    event = "windows_high_resolution_timer_unavailable",
                    error = ?high_resolution_error,
                    "High-resolution waitable timer is unavailable; using a standard waitable timer"
                );
                let handle =
                    unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), 0, TIMER_ALL_ACCESS.0) }
                        .with_context(|| "Failed to create Windows capture waitable timer")?;
                Ok(Self {
                    handle,
                    high_resolution: false,
                })
            }
        }
    }

    fn wait(&self, duration: Duration) -> eros::Result<()> {
        let due_time_100ns = duration
            .as_nanos()
            .div_ceil(100)
            .max(1)
            .min(i64::MAX as u128) as i64;
        let due_time = -due_time_100ns;
        unsafe { SetWaitableTimerEx(self.handle, &due_time, 0, None, None, None, 0) }
            .with_context(|| "Failed to arm Windows capture waitable timer")?;
        let result = unsafe { WaitForSingleObject(self.handle, INFINITE) };
        if result != WAIT_OBJECT_0 {
            eros::bail!("Windows capture waitable timer returned {result:?}");
        }
        Ok(())
    }
}

impl Drop for HighResolutionWaitableTimer {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

struct FixedCaptureClock {
    period: Duration,
    next_frame_at: Instant,
}

impl FixedCaptureClock {
    fn new(frame_rate: FrameRate) -> Self {
        Self::new_at(frame_rate, Instant::now())
    }

    fn new_at(frame_rate: FrameRate, now: Instant) -> Self {
        let numerator = u128::from(frame_rate.numerator().max(1));
        let denominator = u128::from(frame_rate.denominator().max(1));
        let nanoseconds = 1_000_000_000_u128
            .saturating_mul(denominator)
            .div_ceil(numerator)
            .min(u64::MAX.into()) as u64;
        let period = Duration::from_nanos(nanoseconds.max(1));
        Self {
            period,
            next_frame_at: now.checked_add(period).unwrap_or(now),
        }
    }

    fn delay(&self) -> Duration {
        self.delay_at(Instant::now())
    }

    fn delay_at(&self, now: Instant) -> Duration {
        self.next_frame_at.saturating_duration_since(now)
    }

    fn advance(&mut self) {
        self.advance_at(Instant::now());
    }

    fn advance_at(&mut self, now: Instant) {
        let next = self.next_frame_at.checked_add(self.period).unwrap_or(now);
        self.next_frame_at = if next > now {
            next
        } else {
            now.checked_add(self.period).unwrap_or(now)
        };
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
        for attempt in 1..=2 {
            match unsafe { output5.DuplicateOutput1(device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) } {
                Ok(duplication) => return Ok(duplication),
                Err(error) if attempt == 1 => {
                    warn!(
                        event = "windows_desktop_duplication_create_retry",
                        attempt,
                        error = ?error,
                        "Retrying Desktop Duplication creation after the previous stream released it"
                    );
                    thread::sleep(Duration::from_millis(200));
                }
                Err(error) => {
                    Err(error).with_context(|| "IDXGIOutput5::DuplicateOutput1 failed")?;
                }
            }
        }
        unreachable!("Desktop Duplication creation loop must return")
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

fn replace_latest<T>(sender: flume::Sender<T>, stale: flume::Receiver<T>, mut value: T) {
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
        fixed_rate_paced: false,
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::kernel::{geometry::FrameRate, video_encoder::VideoFrameRateMode};

    use super::{FixedCaptureClock, WindowsCaptureBackend, WindowsScreenCaptureManagerState};

    #[test]
    fn fixed_capture_clock_keeps_absolute_deadlines() {
        let start = Instant::now();
        let mut clock =
            FixedCaptureClock::new_at(FrameRate::new(100, 1).expect("frame rate"), start);

        assert_eq!(
            clock.delay_at(start + Duration::from_millis(4)),
            Duration::from_millis(6)
        );
        clock.advance_at(start + Duration::from_millis(6));
        assert_eq!(
            clock.delay_at(start + Duration::from_millis(6)),
            Duration::from_millis(14)
        );
    }

    #[test]
    fn fixed_capture_clock_skips_missed_deadlines_without_bursting() {
        let start = Instant::now();
        let mut clock =
            FixedCaptureClock::new_at(FrameRate::new(100, 1).expect("frame rate"), start);

        clock.advance_at(start + Duration::from_millis(35));

        assert_eq!(
            clock.delay_at(start + Duration::from_millis(35)),
            Duration::from_millis(10)
        );
    }

    #[test]
    fn capture_frame_schedule_is_consumed_once() {
        let state = WindowsScreenCaptureManagerState::new(
            WindowsCaptureBackend::DesktopDuplication,
            false,
            Duration::from_secs(2),
        );
        let target = FrameRate::new(120, 1).expect("target frame rate");
        let source = FrameRate::new(60, 1).expect("source frame rate");

        state.set_next_frame_schedule(VideoFrameRateMode::Fixed, target);
        let configured = state.take_next_frame_schedule(source);
        let fallback = state.take_next_frame_schedule(source);

        assert_eq!(configured.frame_rate_mode, VideoFrameRateMode::Fixed);
        assert_eq!(configured.frame_rate, target);
        assert_eq!(fallback.frame_rate_mode, VideoFrameRateMode::Dynamic);
        assert_eq!(fallback.frame_rate, source);
    }
}
