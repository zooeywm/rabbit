use std::{sync::OnceLock, time::Duration};

use eros::Context as _;
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use tracing::info;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HINSTANCE, HWND, RECT},
        Graphics::{
            Direct3D11::{
                D3D11_BIND_DECODER, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
                ID3D11Texture2D,
            },
            DirectComposition::{
                COMPOSITIONOBJECT_READ, COMPOSITIONOBJECT_WRITE, DCompositionCreateDevice,
                DCompositionCreateSurfaceHandle, IDCompositionDevice, IDCompositionTarget,
                IDCompositionVisual,
            },
            Dxgi::Common::DXGI_FORMAT_NV12,
            Dxgi::{
                DXGI_DECODE_SWAP_CHAIN_DESC, DXGI_PRESENT, IDXGIDecodeSwapChain, IDXGIDevice,
                IDXGIFactoryMedia, IDXGIResource,
            },
            Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_BOTTOM, HWND_TOP, IsIconic,
            IsWindowVisible, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            SetWindowPos, ShowWindow, WNDCLASSEXW, WS_CHILD, WS_EX_NOREDIRECTIONBITMAP, WS_VISIBLE,
        },
    },
    core::{Interface as _, w},
};

use crate::{
    infra::platform::video_decoder::WindowsDecodedFrame,
    kernel::video_renderer::{VideoRenderer, VideoViewport},
};

use super::{
    client_video_probe::ClientVideoProbeReporter, video_color::decode_swap_chain_color_space,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NativeVideoViewport {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl From<VideoViewport> for NativeVideoViewport {
    fn from(value: VideoViewport) -> Self {
        Self {
            x: value.x.min(i32::MAX as u32) as i32,
            y: value.y.min(i32::MAX as u32) as i32,
            width: value.width.min(i32::MAX as u32) as i32,
            height: value.height.min(i32::MAX as u32) as i32,
        }
    }
}

pub(crate) struct NativeVideoRenderer {
    main_window: HWND,
    video_window: HWND,
    background_window: HWND,
    viewport: NativeVideoViewport,
    pending_frame: Option<WindowsDecodedFrame>,
    current_frame: Option<WindowsDecodedFrame>,
    composition: Option<DecodeComposition>,
    probe_reporter: ClientVideoProbeReporter,
}

impl NativeVideoRenderer {
    pub(crate) fn new(window: &slint::Window, probe_interval: Duration) -> eros::Result<Self> {
        let window_handle = window.window_handle();
        let handle = window_handle
            .window_handle()
            .with_context(|| "Slint did not expose a Windows window handle")?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            eros::bail!("Slint window is not backed by a Win32 HWND");
        };
        let main_window = HWND(handle.hwnd.get() as *mut _);
        let (video_window, background_window) = create_video_windows(main_window)?;
        info!(
            event = "windows_video_hwnd_created",
            "Created native child video HWND"
        );
        Ok(Self {
            main_window,
            video_window,
            background_window,
            viewport: NativeVideoViewport::default(),
            pending_frame: None,
            current_frame: None,
            composition: None,
            probe_reporter: ClientVideoProbeReporter::new(probe_interval),
        })
    }

    pub(crate) fn set_viewport(&mut self, viewport: NativeVideoViewport) -> eros::Result<()> {
        if viewport.width < 0 || viewport.height < 0 {
            eros::bail!(
                "Windows native video viewport has negative size {}x{}",
                viewport.width,
                viewport.height
            );
        }
        self.viewport = viewport;
        Ok(())
    }

    pub(crate) fn validate_frame(&self, frame: &WindowsDecodedFrame) -> eros::Result<()> {
        if frame.size.width == 0 || frame.size.height == 0 {
            eros::bail!("Windows decoded frame has an empty size");
        }
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { frame.texture.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_NV12 {
            eros::bail!(
                "Windows decoded texture has format {:?}, expected NV12",
                desc.Format
            );
        }
        if desc.BindFlags & D3D11_BIND_DECODER.0 as u32 == 0 {
            eros::bail!("Windows decoded texture is not bound as a decoder surface");
        }
        if frame.subresource_index >= desc.ArraySize {
            eros::bail!(
                "Windows decoded subresource {} exceeds texture array size {}",
                frame.subresource_index,
                desc.ArraySize
            );
        }
        if frame.source_x.saturating_add(frame.size.width) > desc.Width
            || frame.source_y.saturating_add(frame.size.height) > desc.Height
        {
            eros::bail!(
                "Windows decoded source {}x{}+{},{} exceeds texture {}x{}",
                frame.size.width,
                frame.size.height,
                frame.source_x,
                frame.source_y,
                desc.Width,
                desc.Height
            );
        }
        Ok(())
    }

    pub(crate) fn teardown(&mut self) -> eros::Result<()> {
        self.clear()?;
        self.composition = None;
        self.probe_reporter.finish();
        Ok(unsafe { DestroyWindow(self.video_window) }
            .with_context(|| "Failed to destroy Windows video window")?)
    }

    fn sync_video_window(&mut self) -> eros::Result<bool> {
        let visible = self.viewport.width > 0
            && self.viewport.height > 0
            && unsafe { IsWindowVisible(self.main_window).as_bool() }
            && !unsafe { IsIconic(self.main_window).as_bool() };
        if !visible {
            let _ = unsafe { ShowWindow(self.video_window, SW_HIDE) };
            return Ok(false);
        }
        unsafe {
            SetWindowPos(
                self.video_window,
                Some(HWND_TOP),
                self.viewport.x,
                self.viewport.y,
                self.viewport.width,
                self.viewport.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .with_context(|| "Failed to position Windows child video window")?;
        unsafe {
            SetWindowPos(
                self.background_window,
                Some(HWND_BOTTOM),
                0,
                0,
                self.viewport.width,
                self.viewport.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .with_context(|| "Failed to size Windows video background window")?;
        Ok(true)
    }
}

impl VideoRenderer for NativeVideoRenderer {
    type Frame = WindowsDecodedFrame;

    fn set_viewport(&mut self, viewport: VideoViewport) {
        self.viewport = viewport.into();
    }

    fn present(&mut self, mut frame: Self::Frame) {
        if let Some(probe) = &mut frame.probe {
            probe.mark_gui_received();
        }
        self.pending_frame = Some(frame);
    }

    fn render(&mut self) -> eros::Result<()> {
        if !self.sync_video_window()? {
            return Ok(());
        }
        let Some(mut frame) = self.pending_frame.take() else {
            return Ok(());
        };
        self.validate_frame(&frame)?;
        let recreate = self
            .composition
            .as_ref()
            .is_none_or(|composition| !composition.uses_resource(&frame.texture));
        if recreate {
            self.composition = None;
            self.composition = Some(DecodeComposition::new(self.video_window, &frame.texture)?);
        }
        let target = fitted_target(
            self.viewport.width as u32,
            self.viewport.height as u32,
            frame.size,
        )?;
        if let Some(probe) = &mut frame.probe {
            probe.mark_render_started();
        }
        self.composition
            .as_mut()
            .with_context(|| "Windows video composition disappeared")?
            .present(
                &frame,
                target,
                self.viewport.width as u32,
                self.viewport.height as u32,
            )?;
        if let Some(mut probe) = frame.probe.take() {
            probe.mark_render_completed();
            self.probe_reporter.record_frame(frame.screen_id, probe);
        }
        self.current_frame = Some(frame);
        Ok(())
    }

    fn clear(&mut self) -> eros::Result<()> {
        self.pending_frame = None;
        self.current_frame = None;
        let _ = unsafe { ShowWindow(self.video_window, SW_HIDE) };
        Ok(())
    }
}

struct DecodeComposition {
    device: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    _surface_content: windows::core::IUnknown,
    surface_handle: windows::Win32::Foundation::HANDLE,
    resource: IDXGIResource,
    swap_chain: IDXGIDecodeSwapChain,
    presented: bool,
    content_probed: bool,
}

impl DecodeComposition {
    fn new(video_window: HWND, texture: &ID3D11Texture2D) -> eros::Result<Self> {
        info!(
            event = "windows_video_composition_creating",
            "Creating DirectComposition decode swap chain"
        );
        let d3d = unsafe { texture.GetDevice() }
            .with_context(|| "Failed to get decoded texture D3D11 device")?;
        let dxgi: IDXGIDevice = d3d
            .cast()
            .with_context(|| "Decoded texture device does not expose IDXGIDevice")?;
        let device: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi) }
            .with_context(|| "Failed to create DirectComposition video device")?;
        let target = unsafe { device.CreateTargetForHwnd(video_window, true) }
            .with_context(|| "Failed to bind DirectComposition to video HWND")?;
        let visual = unsafe { device.CreateVisual() }
            .with_context(|| "Failed to create DirectComposition video visual")?;
        let surface_handle = unsafe {
            DCompositionCreateSurfaceHandle(
                (COMPOSITIONOBJECT_READ | COMPOSITIONOBJECT_WRITE) as u32,
                None,
            )
        }
        .with_context(|| "Failed to create DirectComposition video surface handle")?;

        let resource: IDXGIResource = texture
            .cast()
            .with_context(|| "Decoded texture does not expose IDXGIResource")?;
        let adapter = unsafe { dxgi.GetAdapter() }?;
        let factory: IDXGIFactoryMedia = unsafe { adapter.GetParent() }?;
        let swap_chain = unsafe {
            factory.CreateDecodeSwapChainForCompositionSurfaceHandle(
                &d3d,
                Some(surface_handle),
                &DXGI_DECODE_SWAP_CHAIN_DESC::default(),
                &resource,
                None::<&windows::Win32::Graphics::Dxgi::IDXGIOutput>,
            )
        }
        .with_context(|| "Failed to create zero-copy DXGI decode swap chain")?;
        unsafe { swap_chain.SetColorSpace(decode_swap_chain_color_space()) }
            .with_context(|| "Failed to set Windows decode swap-chain color space")?;
        let surface_content = unsafe { device.CreateSurfaceFromHandle(surface_handle) }
            .with_context(|| "Failed to wrap DirectComposition video surface")?;
        unsafe {
            visual.SetContent(&surface_content)?;
            target.SetRoot(&visual)?;
            device.Commit()?;
            device.WaitForCommitCompletion()?;
        }
        info!(
            event = "windows_video_composition_created",
            "Created DirectComposition decode swap chain"
        );
        Ok(Self {
            device,
            _target: target,
            _visual: visual,
            _surface_content: surface_content,
            surface_handle,
            resource,
            swap_chain,
            presented: false,
            content_probed: false,
        })
    }

    fn uses_resource(&self, texture: &ID3D11Texture2D) -> bool {
        texture.cast::<IDXGIResource>().is_ok_and(|resource| {
            windows::core::Interface::as_raw(&resource)
                == windows::core::Interface::as_raw(&self.resource)
        })
    }

    fn present(
        &mut self,
        frame: &WindowsDecodedFrame,
        target: RECT,
        destination_width: u32,
        destination_height: u32,
    ) -> eros::Result<()> {
        let source = RECT {
            left: frame.source_x as i32,
            top: frame.source_y as i32,
            right: frame.source_x.saturating_add(frame.size.width) as i32,
            bottom: frame.source_y.saturating_add(frame.size.height) as i32,
        };
        if !self.content_probed {
            match inspect_nv12_luma(frame) {
                Ok(luma) => info!(
                    event = "windows_video_first_frame_luma",
                    samples = luma.samples,
                    minimum = luma.minimum,
                    maximum = luma.maximum,
                    average = luma.average,
                    "Sampled first decoded NV12 frame luminance"
                ),
                Err(error) => tracing::warn!(
                    event = "windows_video_first_frame_luma_failed",
                    error = %error,
                    "Failed to sample first decoded NV12 frame luminance"
                ),
            }
            self.content_probed = true;
        }
        if !self.presented {
            info!(
                event = "windows_video_first_present_pending",
                subresource_index = frame.subresource_index,
                source_width = frame.size.width,
                source_height = frame.size.height,
                source_left = source.left,
                source_top = source.top,
                destination_width,
                destination_height,
                target_left = target.left,
                target_top = target.top,
                target_right = target.right,
                target_bottom = target.bottom,
                "Presenting first D3D11 decoder surface"
            );
        }
        unsafe {
            self.swap_chain
                .SetDestSize(destination_width, destination_height)
        }
        .with_context(|| "Failed to resize Windows decode swap-chain destination")?;
        unsafe { self.swap_chain.SetSourceRect(&source) }
            .with_context(|| "Failed to set Windows decode swap-chain source rectangle")?;
        unsafe { self.swap_chain.SetTargetRect(&target) }
            .with_context(|| "Failed to set Windows decode swap-chain target rectangle")?;
        if !self.presented {
            let applied_source = unsafe { self.swap_chain.GetSourceRect() }
                .with_context(|| "Failed to read Windows decode swap-chain source rectangle")?;
            let applied_target = unsafe { self.swap_chain.GetTargetRect() }
                .with_context(|| "Failed to read Windows decode swap-chain target rectangle")?;
            let mut applied_width = 0;
            let mut applied_height = 0;
            unsafe {
                self.swap_chain
                    .GetDestSize(&mut applied_width, &mut applied_height)
            }
            .with_context(|| "Failed to read Windows decode swap-chain destination size")?;
            info!(
                event = "windows_video_first_swap_chain_state",
                source_left = applied_source.left,
                source_top = applied_source.top,
                source_right = applied_source.right,
                source_bottom = applied_source.bottom,
                target_left = applied_target.left,
                target_top = applied_target.top,
                target_right = applied_target.right,
                target_bottom = applied_target.bottom,
                destination_width = applied_width,
                destination_height = applied_height,
                color_space = ?unsafe { self.swap_chain.GetColorSpace() },
                "Verified first Windows decode swap-chain state"
            );
        }
        unsafe {
            self.swap_chain
                .PresentBuffer(frame.subresource_index, 0, DXGI_PRESENT(0))
                .ok()
        }
        .with_context(|| "Failed to present Windows decode swap-chain buffer")?;
        unsafe { self.device.Commit() }
            .with_context(|| "Failed to commit Windows DirectComposition frame")?;
        if !self.presented {
            info!(
                event = "windows_video_first_presented",
                "Presented first D3D11 decoder surface"
            );
            self.presented = true;
        }
        Ok(())
    }
}

struct LumaProbe {
    samples: u64,
    minimum: u8,
    maximum: u8,
    average: u64,
}

fn inspect_nv12_luma(frame: &WindowsDecodedFrame) -> eros::Result<LumaProbe> {
    let device = unsafe { frame.texture.GetDevice() }
        .with_context(|| "Failed to get D3D11 device for NV12 inspection")?;
    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { frame.texture.GetDesc(&mut source_desc) };
    let staging_desc = D3D11_TEXTURE2D_DESC {
        MipLevels: 1,
        ArraySize: 1,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
        ..source_desc
    };
    let mut staging = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
        .with_context(|| "Failed to create staging NV12 texture")?;
    let staging = staging.with_context(|| "D3D11 returned no staging NV12 texture")?;
    let context = unsafe { device.GetImmediateContext() }
        .with_context(|| "Failed to get D3D11 context for NV12 inspection")?;
    unsafe {
        context.CopySubresourceRegion(
            &staging,
            0,
            0,
            0,
            0,
            &frame.texture,
            frame.subresource_index,
            None,
        );
        context.Flush();
    }
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .with_context(|| "Failed to map staging NV12 texture")?;

    let mut minimum = u8::MAX;
    let mut maximum = u8::MIN;
    let mut total = 0u64;
    let mut samples = 0u64;
    let step = 8usize;
    for y in (frame.source_y..frame.source_y.saturating_add(frame.size.height)).step_by(step) {
        let row = unsafe { (mapped.pData as *const u8).add(y as usize * mapped.RowPitch as usize) };
        for x in (frame.source_x..frame.source_x.saturating_add(frame.size.width)).step_by(step) {
            let value = unsafe { *row.add(x as usize) };
            minimum = minimum.min(value);
            maximum = maximum.max(value);
            total = total.saturating_add(u64::from(value));
            samples = samples.saturating_add(1);
        }
    }
    unsafe { context.Unmap(&staging, 0) };
    if samples == 0 {
        eros::bail!("Decoded NV12 frame produced no luminance samples");
    }
    Ok(LumaProbe {
        samples,
        minimum,
        maximum,
        average: total / samples,
    })
}

impl Drop for DecodeComposition {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.surface_handle) };
    }
}

fn fitted_target(
    width: u32,
    height: u32,
    frame_size: crate::kernel::geometry::PixelSize,
) -> eros::Result<RECT> {
    if width == 0 || height == 0 || frame_size.width == 0 || frame_size.height == 0 {
        eros::bail!("Cannot fit an empty Windows video viewport");
    }
    let scale_by_width = u64::from(width) * u64::from(frame_size.height)
        <= u64::from(height) * u64::from(frame_size.width);
    let (fitted_width, fitted_height) = if scale_by_width {
        (
            width,
            (u64::from(width) * u64::from(frame_size.height) / u64::from(frame_size.width)) as u32,
        )
    } else {
        (
            (u64::from(height) * u64::from(frame_size.width) / u64::from(frame_size.height)) as u32,
            height,
        )
    };
    let fitted = VideoViewport {
        x: (width - fitted_width) / 2,
        y: (height - fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
    };
    Ok(RECT {
        left: fitted.x as i32,
        top: fitted.y as i32,
        right: fitted.x.saturating_add(fitted.width) as i32,
        bottom: fitted.y.saturating_add(fitted.height) as i32,
    })
}

fn create_video_windows(main_window: HWND) -> eros::Result<(HWND, HWND)> {
    register_video_window_class()?;
    let module = unsafe { GetModuleHandleW(None) }?;
    let video_window = unsafe {
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("RabbitVideoWindow"),
            w!("Rabbit Video"),
            WS_CHILD,
            0,
            0,
            1,
            1,
            Some(main_window),
            None,
            Some(HINSTANCE(module.0)),
            None,
        )
    }
    .with_context(|| "Failed to create Windows native video HWND")?;
    let background_window = match unsafe {
        CreateWindowExW(
            Default::default(),
            w!("RabbitVideoWindow"),
            w!("Rabbit Video Background"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            Some(video_window),
            None,
            Some(HINSTANCE(module.0)),
            None,
        )
    } {
        Ok(window) => window,
        Err(error) => {
            let _ = unsafe { DestroyWindow(video_window) };
            Err::<HWND, _>(error)
                .with_context(|| "Failed to create Windows video background child HWND")?
        }
    };
    Ok((video_window, background_window))
}

fn register_video_window_class() -> eros::Result<()> {
    static REGISTRATION: OnceLock<Result<(), String>> = OnceLock::new();
    match REGISTRATION
        .get_or_init(|| {
            let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
            let brush = unsafe { GetStockObject(BLACK_BRUSH) };
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(video_window_proc),
                hInstance: HINSTANCE(module.0),
                hbrBackground: HBRUSH(brush.0),
                lpszClassName: w!("RabbitVideoWindow"),
                ..Default::default()
            };
            if unsafe { RegisterClassExW(&class) } == 0 {
                return Err(windows::core::Error::from_thread().to_string());
            }
            Ok(())
        })
        .clone()
    {
        Ok(()) => Ok(()),
        Err(message) => eros::bail!("Failed to register Windows video window: {}", message),
    }
}

unsafe extern "system" fn video_window_proc(
    window: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
