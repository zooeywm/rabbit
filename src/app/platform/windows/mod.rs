use std::{future::Future, time::Duration};

use slint::ComponentHandle as _;
use tracing::info;

mod record;

use crate::{
    app::{
        App,
        config::Config,
        gui::{RabbitWindow, VideoViewStack},
    },
    infra::{
        ConnectionEndpoint, NativeVideoRenderer, NativeVideoViewport, OpenGlVideoRenderer,
        WgcFramePipelineManagerState, WgcScreenCaptureManagerState, WindowsDecodedFrame,
        WindowsRemoteInputInjector, WindowsScreenLayoutManagerState, WindowsVideoDecoder,
        WindowsVideoEncoder, WorkerReaper, WorkerReaperHandle,
    },
    kernel::{session::ReceivedVideoFrame, video_renderer::VideoRenderer as _},
};

pub(crate) use crate::app::stack::{ApplicationStack, RemoteVideoStack, RunnableApp};

enum WindowsApplicationStack {
    WgcD3d11,
}

pub(crate) struct WgcD3d11ApplicationStack;

#[cfg(test)]
pub(crate) use WgcD3d11ApplicationStack as TestApplicationStack;

pub(crate) fn run(config: Config) -> eros::Result<()> {
    match select_application_stack(&config) {
        WindowsApplicationStack::WgcD3d11 => {
            crate::app::gui::run::<WgcD3d11ApplicationStack>(config)
        }
    }
}

pub(crate) fn run_headless(config: Config) -> eros::Result<()> {
    match select_application_stack(&config) {
        WindowsApplicationStack::WgcD3d11 => {
            let runtime =
                compio::runtime::Runtime::new().expect("Headless Compio runtime should start");
            runtime.block_on(crate::app::headless::run::<WgcD3d11ApplicationStack>(
                config,
            ))
        }
    }
}

pub(crate) fn run_record(
    config: Config,
    options: crate::app::cli::RecordOptions,
) -> eros::Result<()> {
    let _ = select_application_stack(&config);
    record::run(config, options)
}

pub(crate) fn install_shutdown_handlers() {}

/// Selects the Windows media backend.
///
/// The WGC + D3D11 + Media Foundation stack is the only wired option and trails
/// the Linux product path in feature completeness. Keep parity work behind this
/// stack boundary.
fn select_application_stack(_config: &Config) -> WindowsApplicationStack {
    info!(
        event = "app_platform_stack_selected",
        stack = WgcD3d11ApplicationStack::name(),
        "Selected Windows application backend stack"
    );
    WindowsApplicationStack::WgcD3d11
}

impl ApplicationStack for WgcD3d11ApplicationStack {
    type App = App<
        WindowsScreenLayoutManagerState,
        WgcScreenCaptureManagerState,
        WgcFramePipelineManagerState,
    >;
    type RemoteVideo = RemoteVideo;
    type RemoteVideoViewStack = RemoteVideoViewStack;
    type ScreenStreamEncoder = WindowsVideoEncoder;
    type RemoteInputInjector = WindowsRemoteInputInjector;

    fn name() -> &'static str {
        "windows-wgc-d3d11-mf"
    }

    fn create_app(
        config: Config,
        connection_endpoint: ConnectionEndpoint,
        worker_reaper: WorkerReaper,
        worker_reaper_handle: WorkerReaperHandle,
    ) -> eros::Result<Self::App> {
        let screen_layout_manager_state = crate::infra::create_screen_layout_manager_state()?;
        let screen_capture_manager_state = crate::infra::create_screen_capture_manager_state(
            config.video.enable_host_probing,
            Duration::from_millis(config.video.probe_interval_ms),
            worker_reaper_handle.clone(),
        );
        let frame_pipeline_manager_state =
            crate::infra::create_frame_pipeline_manager_state(worker_reaper_handle);

        Ok(App::new(
            config,
            screen_layout_manager_state,
            screen_capture_manager_state,
            frame_pipeline_manager_state,
            connection_endpoint,
            worker_reaper,
        ))
    }

    fn create_remote_input_injector() -> Self::RemoteInputInjector {
        WindowsRemoteInputInjector::new()
    }
}

pub(crate) struct RemoteVideoViewStack;

impl VideoViewStack for RemoteVideoViewStack {
    type Frame = WindowsDecodedFrame;
    type NativeRenderer = NativeVideoRenderer;
    type OpenGlRenderer = OpenGlVideoRenderer;
    type NativeViewport = NativeVideoViewport;

    fn select_slint_backend() -> eros::Result<()> {
        let mut settings = slint::wgpu_29::WGPUSettings::default();
        settings.backends = slint::wgpu_29::wgpu::Backends::DX12;
        slint::BackendSelector::new()
            .with_winit_window_attributes_hook(|attributes| attributes.with_transparent(false))
            .require_wgpu_29(slint::wgpu_29::WGPUConfiguration::Automatic(settings))
            .select()
            .map_err(Into::into)
    }

    fn create_native_renderer(
        window: &slint::Window,
        probe_interval: Duration,
    ) -> eros::Result<Self::NativeRenderer> {
        NativeVideoRenderer::new(window, probe_interval)
    }

    fn create_opengl_renderer(
        get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void,
        probe_interval: Duration,
    ) -> eros::Result<Self::OpenGlRenderer> {
        OpenGlVideoRenderer::new(get_proc_address, probe_interval)
    }

    fn set_native_viewport(
        renderer: &mut Self::NativeRenderer,
        viewport: Self::NativeViewport,
    ) -> eros::Result<()> {
        renderer.set_viewport(viewport)
    }

    fn validate_native_frame(
        renderer: &Self::NativeRenderer,
        frame: &Self::Frame,
    ) -> eros::Result<()> {
        renderer.validate_frame(frame)
    }

    fn present_native_frame(renderer: &mut Self::NativeRenderer, frame: Self::Frame) {
        renderer.present(frame);
    }

    fn render_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()> {
        renderer.render()
    }

    fn clear_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()> {
        renderer.clear()
    }

    fn teardown_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()> {
        renderer.teardown()
    }

    fn teardown_opengl_renderer(renderer: &mut Self::OpenGlRenderer) -> eros::Result<()> {
        renderer.teardown()
    }

    fn native_viewport(window: &RabbitWindow, visible: bool) -> eros::Result<Self::NativeViewport> {
        if !visible {
            return Ok(NativeVideoViewport {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        }
        let scale = window.window().scale_factor();
        Ok(NativeVideoViewport {
            x: physical_pixels(window.get_video_viewport_x(), scale)?.min(i32::MAX as u32) as i32,
            y: physical_pixels(window.get_video_viewport_y(), scale)?.min(i32::MAX as u32) as i32,
            width: physical_pixels(window.get_video_viewport_width(), scale)?.min(i32::MAX as u32)
                as i32,
            height: physical_pixels(window.get_video_viewport_height(), scale)?.min(i32::MAX as u32)
                as i32,
        })
    }
}

fn physical_pixels(logical: f32, scale: f32) -> eros::Result<u32> {
    let physical = logical * scale;
    if !physical.is_finite() || physical < 0.0 || physical > u32::MAX as f32 {
        eros::bail!("Invalid physical video viewport coordinate {}", physical);
    }
    Ok(physical.round() as u32)
}

pub(crate) struct RemoteVideo;

impl RemoteVideoStack for RemoteVideo {
    type Decoder = WindowsVideoDecoder;
    type Frame = WindowsDecodedFrame;

    fn run_decoder<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
        enable_probing: bool,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        WindowsVideoDecoder::run_with_probing(inputs, present_frame, enable_probing)
    }
}

mod deps;
