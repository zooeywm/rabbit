use std::{
    future::Future,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use tracing::info;

mod record;

use crate::{
    app::{
        App,
        config::Config,
        gui::{RabbitWindow, VideoViewStack},
        platform::{ApplicationStack, RemoteVideoStack},
    },
    infra::{
        ConnectionEndpoint, GStreamerDecodedFrame, GStreamerVideoDecoder, GStreamerVideoEncoder,
        GbmFramePipelineManagerState, KmsScreenCaptureManagerState, LinuxAbsolutePointerInjector,
        NativeVideoRenderer, NativeVideoViewport, NiriScreenLayoutManagerState,
        OpenGlVideoRenderer, WorkerReaper, WorkerReaperHandle,
    },
    kernel::session::ReceivedVideoFrame,
};

enum LinuxApplicationStack {
    NiriKmsGbm,
}

pub(crate) struct NiriKmsGbmApplicationStack;

#[cfg(test)]
pub(crate) use NiriKmsGbmApplicationStack as TestApplicationStack;

pub(crate) fn run(config: Config) -> eros::Result<()> {
    match select_application_stack(&config) {
        LinuxApplicationStack::NiriKmsGbm => {
            crate::app::gui::run::<NiriKmsGbmApplicationStack>(config)
        }
    }
}

pub(crate) fn run_headless(config: Config) -> eros::Result<()> {
    match select_application_stack(&config) {
        LinuxApplicationStack::NiriKmsGbm => {
            let runtime =
                compio::runtime::Runtime::new().expect("Headless Compio runtime should start");
            runtime.block_on(crate::app::headless::run::<NiriKmsGbmApplicationStack>(
                config,
            ))
        }
    }
}

pub(crate) fn run_record(
    config: Config,
    options: crate::app::cli::RecordOptions,
) -> eros::Result<()> {
    match select_application_stack(&config) {
        LinuxApplicationStack::NiriKmsGbm => {
            let runtime =
                compio::runtime::Runtime::new().expect("Record Compio runtime should start");
            runtime.block_on(record::run::<NiriKmsGbmApplicationStack>(config, options))
        }
    }
}

pub(crate) fn install_shutdown_handlers() {
    static SIGNAL_COUNT: AtomicU32 = AtomicU32::new(0);

    // SAFETY: handler only touches atomics and may call `_exit` (async-signal-safe).
    unsafe extern "C" fn on_stop_signal(_: libc::c_int) {
        let count = SIGNAL_COUNT.fetch_add(1, Ordering::SeqCst);
        if count >= 1 {
            unsafe { libc::_exit(130) };
        }
        crate::app::shutdown::request();
    }

    unsafe {
        let handler = on_stop_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

/// Selects the Linux media backend.
///
/// Today only the niri + KMS + GBM + GStreamer stack is wired. Additional
/// stacks should become new `LinuxApplicationStack` variants rather than
/// inlining platform choice into session or GUI code.
fn select_application_stack(_config: &Config) -> LinuxApplicationStack {
    let niri_socket_present = std::env::var_os("NIRI_SOCKET").is_some();
    info!(
        event = "app_platform_stack_selected",
        stack = NiriKmsGbmApplicationStack::name(),
        niri_socket_present,
        "Selected Linux application backend stack"
    );
    LinuxApplicationStack::NiriKmsGbm
}

impl ApplicationStack for NiriKmsGbmApplicationStack {
    type App = App<
        NiriScreenLayoutManagerState,
        KmsScreenCaptureManagerState,
        GbmFramePipelineManagerState,
    >;
    type RemoteVideo = RemoteVideo;
    type RemoteVideoViewStack = RemoteVideoViewStack;
    type ScreenStreamEncoder = GStreamerVideoEncoder;
    type AbsolutePointerInjector = LinuxAbsolutePointerInjector;

    fn name() -> &'static str {
        "linux/niri-kms-gbm-gstreamer-wayland"
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

    fn create_absolute_pointer_injector() -> Self::AbsolutePointerInjector {
        LinuxAbsolutePointerInjector::new()
    }
}

pub(crate) struct RemoteVideoViewStack;

impl VideoViewStack for RemoteVideoViewStack {
    type Frame = GStreamerDecodedFrame;
    type NativeRenderer = NativeVideoRenderer;
    type OpenGlRenderer = OpenGlVideoRenderer;
    type NativeViewport = NativeVideoViewport;

    fn select_slint_backend() -> eros::Result<()> {
        slint::BackendSelector::new()
            .with_winit_window_attributes_hook(|attributes| attributes.with_transparent(true))
            .require_opengl_es_with_version(3, 0)
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
        Ok(NativeVideoViewport {
            x: logical_pixels(window.get_video_viewport_x())?,
            y: logical_pixels(window.get_video_viewport_y())?,
            width: logical_pixels(window.get_video_viewport_width())?,
            height: logical_pixels(window.get_video_viewport_height())?,
        })
    }
}

fn logical_pixels(logical: f32) -> eros::Result<i32> {
    if !logical.is_finite() || logical < 0.0 || logical > i32::MAX as f32 {
        eros::bail!("Invalid logical video viewport coordinate {}", logical);
    }
    Ok(logical.round() as i32)
}

pub(crate) struct RemoteVideo;

impl RemoteVideoStack for RemoteVideo {
    type Decoder = GStreamerVideoDecoder;
    type Frame = GStreamerDecodedFrame;

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
        GStreamerVideoDecoder::run_with_probing(inputs, present_frame, enable_probing)
    }
}

mod deps;
