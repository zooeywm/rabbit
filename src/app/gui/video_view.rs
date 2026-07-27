use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod backend;

use eros::Context as _;
use slint::{ComponentHandle as _, GraphicsAPI, RenderingState};
use tracing::info;

use crate::{
    app::{
        config::VideoDisplayPreference,
        gui::view::{GuiIntent, RabbitWindow},
    },
    kernel::{
        screen_manager::ScreenId,
        session::SessionId,
        video_decoder::DecodedVideoFrame as DecodedVideoFrameTrait,
        video_renderer::{VideoRenderer as _, VideoViewport},
    },
};

use crate::app::gui::video_view::backend::{VideoDisplayBackend, select_video_display_backend};

pub(crate) trait VideoViewStack: 'static {
    type Frame: DecodedVideoFrameTrait + 'static;
    type NativeRenderer: 'static;
    type OpenGlRenderer: crate::kernel::video_renderer::VideoRenderer<Frame = Self::Frame> + 'static;
    type NativeViewport;

    fn create_native_renderer(
        window: &slint::Window,
        probe_interval: Duration,
    ) -> eros::Result<Self::NativeRenderer>;

    fn create_opengl_renderer(
        get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void,
        probe_interval: Duration,
    ) -> eros::Result<Self::OpenGlRenderer>;

    fn set_native_viewport(
        renderer: &mut Self::NativeRenderer,
        viewport: Self::NativeViewport,
    ) -> eros::Result<()>;

    fn validate_native_frame(
        renderer: &Self::NativeRenderer,
        frame: &Self::Frame,
    ) -> eros::Result<()>;

    fn present_native_frame(renderer: &mut Self::NativeRenderer, frame: Self::Frame);
    fn render_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()>;
    fn clear_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()>;
    fn teardown_native_renderer(renderer: &mut Self::NativeRenderer) -> eros::Result<()>;
    fn teardown_opengl_renderer(renderer: &mut Self::OpenGlRenderer) -> eros::Result<()>;
    fn native_viewport(window: &RabbitWindow, visible: bool) -> eros::Result<Self::NativeViewport>;
}

enum VideoViewCommand<Stack>
where
    Stack: VideoViewStack,
{
    Present {
        session_id: SessionId,
        screen_id: ScreenId,
        frame: Box<Stack::Frame>,
    },
    Clear,
}

enum ActiveVideoDisplay<Stack>
where
    Stack: VideoViewStack,
{
    Native(Box<Stack::NativeRenderer>),
    Slint(Box<Stack::OpenGlRenderer>),
}

impl<Stack> ActiveVideoDisplay<Stack>
where
    Stack: VideoViewStack,
{
    fn clear(&mut self) -> eros::Result<()> {
        match self {
            Self::Native(renderer) => Stack::clear_native_renderer(renderer),
            Self::Slint(renderer) => renderer.clear(),
        }
    }

    fn teardown(&mut self) -> eros::Result<()> {
        match self {
            Self::Native(renderer) => Stack::teardown_native_renderer(renderer),
            Self::Slint(renderer) => Stack::teardown_opengl_renderer(renderer),
        }
    }
}

pub(crate) struct VideoViewPublisher<Stack>
where
    Stack: VideoViewStack,
{
    sender: flume::Sender<VideoViewCommand<Stack>>,
    stale: flume::Receiver<VideoViewCommand<Stack>>,
    window: slint::Weak<RabbitWindow>,
    delivery_scheduled: Arc<AtomicBool>,
}

impl<Stack> Clone for VideoViewPublisher<Stack>
where
    Stack: VideoViewStack,
{
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            stale: self.stale.clone(),
            window: self.window.clone(),
            delivery_scheduled: Arc::clone(&self.delivery_scheduled),
        }
    }
}

struct VideoViewState<Stack>
where
    Stack: VideoViewStack,
{
    display: Option<ActiveVideoDisplay<Stack>>,
    active_stream: Option<(SessionId, ScreenId)>,
    failed: bool,
}

pub(crate) fn install<Stack>(
    window: &RabbitWindow,
    errors: flume::Sender<GuiIntent>,
    preference: VideoDisplayPreference,
    probe_interval: Duration,
) -> eros::Result<VideoViewPublisher<Stack>>
where
    Stack: VideoViewStack,
{
    let (sender, receiver) = flume::bounded(1);
    let publisher = VideoViewPublisher {
        sender,
        stale: receiver.clone(),
        window: window.as_weak(),
        delivery_scheduled: Arc::new(AtomicBool::new(false)),
    };
    let weak_window = window.as_weak();
    let view_state = Rc::new(RefCell::new(VideoViewState {
        display: None,
        active_stream: None,
        failed: false,
    }));

    let direct_state = Rc::clone(&view_state);
    let direct_commands = receiver.clone();
    let direct_window = weak_window.clone();
    let direct_errors = errors.clone();
    window.on_video_frame_available(move || {
        let mut state = direct_state.borrow_mut();
        if state.failed {
            return;
        }
        if !matches!(state.display, Some(ActiveVideoDisplay::Native(_))) {
            if let Some(window) = direct_window.upgrade() {
                window.window().request_redraw();
            }
            return;
        }
        let VideoViewState {
            display,
            active_stream,
            ..
        } = &mut *state;
        match render_native_frame(&direct_commands, &direct_window, display, active_stream) {
            Ok(Some((session_id, screen_id))) => {
                let _ = direct_errors.send(GuiIntent::VideoFrameReady {
                    session_id,
                    screen_id,
                });
            }
            Ok(None) => {}
            Err(error) => fail_video_display(&mut state, &direct_errors, error),
        }
    });

    let rendering_state = Rc::clone(&view_state);
    window
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            let mut video = rendering_state.borrow_mut();
            if video.failed {
                return;
            }
            let result = match state {
                RenderingState::RenderingSetup => Ok(()),
                RenderingState::AfterRendering => {
                    let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
                        return report_error_once(
                            &errors,
                            &mut video.failed,
                            "Slint did not provide the required native OpenGL renderer".to_string(),
                        );
                    };
                    let VideoViewState {
                        display,
                        active_stream,
                        ..
                    } = &mut *video;
                    let result = render_video_frame(
                        &receiver,
                        &weak_window,
                        display,
                        active_stream,
                        preference,
                        probe_interval,
                        get_proc_address,
                    );
                    match result {
                        Ok(Some((session_id, screen_id))) => {
                            if errors
                                .send(GuiIntent::VideoFrameReady {
                                    session_id,
                                    screen_id,
                                })
                                .is_err()
                            {
                                return;
                            }
                            Ok(())
                        }
                        Ok(None) => Ok(()),
                        Err(error) => Err(error),
                    }
                }
                RenderingState::RenderingTeardown => match video.display.take() {
                    Some(mut display) => display.teardown(),
                    None => Ok(()),
                },
                RenderingState::BeforeRendering => Ok(()),
                _ => Ok(()),
            };

            if let Err(error) = result {
                fail_video_display(&mut video, &errors, error);
            }
        })
        .context("Failed to install the Slint DMA-BUF video rendering bridge")?;

    Ok(publisher)
}

impl<Stack> VideoViewPublisher<Stack>
where
    Stack: VideoViewStack,
{
    pub(crate) fn present(
        &self,
        session_id: SessionId,
        screen_id: ScreenId,
        frame: Stack::Frame,
    ) -> eros::Result<()>
    where
        Stack::Frame: Send,
    {
        if frame.screen_id() != screen_id {
            eros::bail!(
                "Decoded screen {} frame cannot be presented for screen {}",
                frame.screen_id().0,
                screen_id.0
            );
        }
        self.publish(VideoViewCommand::Present {
            session_id,
            screen_id,
            frame: Box::new(frame),
        })
    }

    pub(crate) fn clear(&self) -> eros::Result<()> {
        self.publish(VideoViewCommand::Clear)
    }

    fn publish(&self, mut command: VideoViewCommand<Stack>) -> eros::Result<()> {
        loop {
            match self.sender.try_send(command) {
                Ok(()) => break,
                Err(flume::TrySendError::Full(returned)) => {
                    command = returned;
                    match self.stale.try_recv() {
                        Ok(_) | Err(flume::TryRecvError::Empty) => {}
                        Err(flume::TryRecvError::Disconnected) => {
                            eros::bail!("Slint video rendering bridge disconnected")
                        }
                    }
                }
                Err(flume::TrySendError::Disconnected(_)) => {
                    eros::bail!("Slint video rendering bridge disconnected")
                }
            }
        }

        if self.delivery_scheduled.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let window = self.window.clone();
        let delivery_scheduled = Arc::clone(&self.delivery_scheduled);
        if let Err(error) = slint::invoke_from_event_loop(move || {
            delivery_scheduled.store(false, Ordering::Relaxed);
            if let Some(window) = window.upgrade() {
                window.invoke_video_frame_available();
            }
        }) {
            self.delivery_scheduled.store(false, Ordering::Relaxed);
            Err::<(), _>(error)
                .context("Failed to deliver a decoded video frame to the GUI event loop")?;
        }
        Ok(())
    }
}

fn render_video_frame<Stack>(
    commands: &flume::Receiver<VideoViewCommand<Stack>>,
    weak_window: &slint::Weak<RabbitWindow>,
    display: &mut Option<ActiveVideoDisplay<Stack>>,
    active_stream: &mut Option<(SessionId, ScreenId)>,
    preference: VideoDisplayPreference,
    probe_interval: Duration,
    get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void,
) -> eros::Result<Option<(SessionId, ScreenId)>>
where
    Stack: VideoViewStack,
{
    let mut presented = None;
    if let Ok(command) = commands.try_recv() {
        match command {
            VideoViewCommand::Present {
                session_id,
                screen_id,
                frame,
            } => {
                let window = weak_window
                    .upgrade()
                    .with_context(|| "Slint window closed before video display initialization")?;
                let initialized_display = display.is_none();
                if display.is_none() {
                    *display = Some(create_video_display(
                        preference,
                        window.window(),
                        get_proc_address,
                        probe_interval,
                    )?);
                }
                present_video_frame(
                    display,
                    preference,
                    get_proc_address,
                    probe_interval,
                    *frame,
                )?;
                if initialized_display && matches!(display, Some(ActiveVideoDisplay::Native(_))) {
                    // Wayland subsurface stacking is latched by the parent
                    // surface's next commit. Schedule exactly one Slint redraw
                    // after native display initialization to apply place_below.
                    window.window().request_redraw();
                }
                if activate_stream(active_stream, session_id, screen_id) {
                    presented = Some((session_id, screen_id));
                }
            }
            VideoViewCommand::Clear => {
                *active_stream = None;
                if let Some(display) = display.as_mut() {
                    display.clear()?;
                }
            }
        }
    }

    let Some(window) = weak_window.upgrade() else {
        return Ok(None);
    };
    let Some(display) = display.as_mut() else {
        return Ok(None);
    };
    if !window.get_video_viewport_visible() || active_stream.is_none() {
        if let ActiveVideoDisplay::Native(renderer) = display {
            Stack::set_native_viewport(renderer, Stack::native_viewport(&window, false)?)?;
            Stack::render_native_renderer(renderer)?;
        }
        return Ok(presented);
    }
    match display {
        ActiveVideoDisplay::Native(renderer) => {
            Stack::set_native_viewport(renderer, Stack::native_viewport(&window, true)?)?;
            Stack::render_native_renderer(renderer)?;
        }
        ActiveVideoDisplay::Slint(renderer) => {
            let scale = window.window().scale_factor();
            renderer.set_viewport(VideoViewport {
                x: physical_pixels(window.get_video_viewport_x(), scale)?,
                y: physical_pixels(window.get_video_viewport_y(), scale)?,
                width: physical_pixels(window.get_video_viewport_width(), scale)?,
                height: physical_pixels(window.get_video_viewport_height(), scale)?,
            });
            renderer.render()?;
        }
    }
    Ok(presented)
}

fn render_native_frame<Stack>(
    commands: &flume::Receiver<VideoViewCommand<Stack>>,
    weak_window: &slint::Weak<RabbitWindow>,
    display: &mut Option<ActiveVideoDisplay<Stack>>,
    active_stream: &mut Option<(SessionId, ScreenId)>,
) -> eros::Result<Option<(SessionId, ScreenId)>>
where
    Stack: VideoViewStack,
{
    let Some(ActiveVideoDisplay::Native(renderer)) = display.as_mut() else {
        return Ok(None);
    };
    let mut presented = None;
    if let Ok(command) = commands.try_recv() {
        match command {
            VideoViewCommand::Present {
                session_id,
                screen_id,
                frame,
            } => {
                Stack::validate_native_frame(renderer, &frame)?;
                Stack::present_native_frame(renderer, *frame);
                if activate_stream(active_stream, session_id, screen_id) {
                    presented = Some((session_id, screen_id));
                }
            }
            VideoViewCommand::Clear => {
                *active_stream = None;
                Stack::clear_native_renderer(renderer)?;
            }
        }
    }
    render_native_viewport::<Stack>(weak_window, renderer, active_stream.is_some())?;
    Ok(presented)
}

fn render_native_viewport<Stack>(
    weak_window: &slint::Weak<RabbitWindow>,
    renderer: &mut Stack::NativeRenderer,
    stream_active: bool,
) -> eros::Result<()>
where
    Stack: VideoViewStack,
{
    let Some(window) = weak_window.upgrade() else {
        return Ok(());
    };
    if !window.get_video_viewport_visible() || !stream_active {
        Stack::set_native_viewport(renderer, Stack::native_viewport(&window, false)?)?;
    } else {
        Stack::set_native_viewport(renderer, Stack::native_viewport(&window, true)?)?;
    }
    Stack::render_native_renderer(renderer)
}

fn create_video_display<Stack>(
    preference: VideoDisplayPreference,
    window: &slint::Window,
    get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void,
    probe_interval: Duration,
) -> eros::Result<ActiveVideoDisplay<Stack>>
where
    Stack: VideoViewStack,
{
    if preference == VideoDisplayPreference::Slint {
        let selection = select_video_display_backend(preference, None)?;
        let display = ActiveVideoDisplay::Slint(Box::new(Stack::create_opengl_renderer(
            get_proc_address,
            probe_interval,
        )?));
        log_video_display_selection(preference, selection.backend, None);
        return Ok(display);
    }

    match Stack::create_native_renderer(window, probe_interval) {
        Ok(renderer) => {
            let selection = select_video_display_backend(preference, None)?;
            log_video_display_selection(preference, selection.backend, None);
            Ok(ActiveVideoDisplay::Native(Box::new(renderer)))
        }
        Err(error) => {
            let reason = format!("{error:?}");
            let selection = select_video_display_backend(preference, Some(reason.clone()))?;
            let display = ActiveVideoDisplay::Slint(Box::new(Stack::create_opengl_renderer(
                get_proc_address,
                probe_interval,
            )?));
            log_video_display_selection(
                preference,
                selection.backend,
                selection.fallback_reason.as_deref(),
            );
            Ok(display)
        }
    }
}

fn present_video_frame<Stack>(
    display: &mut Option<ActiveVideoDisplay<Stack>>,
    preference: VideoDisplayPreference,
    get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void,
    probe_interval: Duration,
    frame: Stack::Frame,
) -> eros::Result<()>
where
    Stack: VideoViewStack,
{
    let error = match display.as_mut() {
        Some(ActiveVideoDisplay::Native(renderer)) => {
            match Stack::validate_native_frame(renderer, &frame) {
                Ok(()) => {
                    Stack::present_native_frame(renderer, frame);
                    return Ok(());
                }
                Err(error) => error,
            }
        }
        Some(ActiveVideoDisplay::Slint(renderer)) => {
            renderer.present(frame);
            return Ok(());
        }
        None => eros::bail!("Video display disappeared before presenting a decoded frame"),
    };

    if preference != VideoDisplayPreference::Auto {
        return Err(error);
    }
    let reason = format!("{error:?}");
    if let Some(mut previous) = display.take() {
        previous
            .teardown()
            .with_context(|| "Failed to tear down rejected native video display")?;
    }
    let mut fallback = Stack::create_opengl_renderer(get_proc_address, probe_interval)
        .with_context(|| "Failed to create Slint video display fallback")?;
    fallback.present(frame);
    *display = Some(ActiveVideoDisplay::Slint(Box::new(fallback)));
    let selection = select_video_display_backend(preference, Some(reason))?;
    log_video_display_selection(
        preference,
        selection.backend,
        selection.fallback_reason.as_deref(),
    );
    Ok(())
}

fn log_video_display_selection(
    preference: VideoDisplayPreference,
    backend: VideoDisplayBackend,
    fallback_reason: Option<&str>,
) {
    info!(
        target: "rabbit::video_display",
        event = "video_display_selected",
        requested = ?preference,
        backend = backend.name(),
        fallback_reason,
        "Selected client video display backend"
    );
}

fn physical_pixels(logical: f32, scale: f32) -> eros::Result<u32> {
    let physical = logical * scale;
    if !physical.is_finite() || physical < 0.0 || physical > u32::MAX as f32 {
        eros::bail!("Invalid physical video viewport coordinate {}", physical);
    }
    Ok(physical.round() as u32)
}

fn activate_stream(
    active_stream: &mut Option<(SessionId, ScreenId)>,
    session_id: SessionId,
    screen_id: ScreenId,
) -> bool {
    let first_frame = *active_stream != Some((session_id, screen_id));
    *active_stream = Some((session_id, screen_id));
    first_frame
}

fn report_error_once(errors: &flume::Sender<GuiIntent>, failed: &mut bool, error: String) {
    if *failed {
        return;
    }
    *failed = true;
    if errors.send(GuiIntent::VideoRendererFailed(error)).is_err()
        && let Err(error) = slint::quit_event_loop()
    {
        eprintln!("Failed to stop Slint after the video renderer failed: {error}");
    }
}

fn fail_video_display<Stack>(
    state: &mut VideoViewState<Stack>,
    errors: &flume::Sender<GuiIntent>,
    error: eros::ErrorUnion,
) where
    Stack: VideoViewStack,
{
    let cleanup_error = state
        .display
        .as_mut()
        .and_then(|display| display.teardown().err());
    state.display = None;
    let mut error = format!("{error:?}");
    if let Some(cleanup_error) = cleanup_error {
        error.push_str(&format!(
            "\nAdditionally failed to release video renderer resources: {cleanup_error:?}"
        ));
    }
    report_error_once(errors, &mut state.failed, error);
}

// Focused hardware test: scripts/test-client-video [positive-seconds]
#[cfg(test)]
mod tests {
    use std::{cell::Cell, pin::Pin, rc::Rc, time::Duration};

    use crate::{
        app::{
            gui::{
                state::{ViewPage, ViewState},
                view::{Gui, GuiIntent, ViewPublisher},
            },
            platform::{ApplicationStack, RemoteVideoStack, TestApplicationStack},
        },
        kernel::{
            screen_manager::ScreenId,
            session::{ReceivedVideoFrame, SessionId},
        },
    };
    use gstreamer::glib::prelude::{Cast as _, ObjectExt as _};
    use gstreamer::prelude::{ElementExt as _, GstBinExtManual as _};

    #[test]
    fn only_the_first_frame_of_an_active_stream_notifies_the_app() {
        let mut active_stream = None;

        assert!(super::activate_stream(
            &mut active_stream,
            SessionId(3),
            ScreenId(1)
        ));
        assert!(!super::activate_stream(
            &mut active_stream,
            SessionId(3),
            ScreenId(1)
        ));
        assert!(super::activate_stream(
            &mut active_stream,
            SessionId(4),
            ScreenId(1)
        ));
    }

    #[test]
    #[ignore = "run through scripts/test-client-video"]
    fn renders_hardware_decoded_dma_bufs_through_the_slint_bridge() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_test_writer()
            .try_init();
        let seconds = std::env::var("RABBIT_CLIENT_VIDEO_TEST_SECONDS")
            .expect("RABBIT_CLIENT_VIDEO_TEST_SECONDS should specify the run duration")
            .parse::<u32>()
            .expect("RABBIT_CLIENT_VIDEO_TEST_SECONDS should be a positive integer");
        assert!(seconds > 0, "Client video test duration should be positive");

        let (gui, publisher, intents) =
            Gui::<<TestApplicationStack as ApplicationStack>::RemoteVideoViewStack>::new(
                crate::app::config::VideoDisplayPreference::Slint,
                Duration::from_secs(2),
            )
            .expect("Slint video test window should be created");
        publisher
            .publish(ViewState {
                page: ViewPage::StreamRequest,
                page_title: "DMA-BUF decode and render test".to_string(),
                page_subtitle: "Waiting for the first decoded frame".to_string(),
                status_text: "Waiting for the first video frame".to_string(),
                stream_title: "Synthetic test stream".to_string(),
                stream_resolution: "1280 × 720".to_string(),
                local_server_online: true,
                ..ViewState::default()
            })
            .expect("Waiting-for-video test state should reach Slint");

        let first_frame_presented = Rc::new(Cell::new(false));
        let renderer_failed = Rc::new(Cell::new(false));
        let event_first_frame_presented = Rc::clone(&first_frame_presented);
        let event_renderer_failed = Rc::clone(&renderer_failed);
        let event_publisher = publisher.clone();
        let event_timer = slint::Timer::default();
        event_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(10),
            move || {
                while let Ok(intent) = intents.try_recv() {
                    match intent {
                        GuiIntent::VideoFrameReady { .. }
                            if !event_first_frame_presented.replace(true) =>
                        {
                            event_publisher
                                .publish(ViewState {
                                    page: ViewPage::Streaming,
                                    page_title: "DMA-BUF decode and render test".to_string(),
                                    page_subtitle:
                                        "Hardware H.264 decoder → EGLImage → Slint OpenGL"
                                            .to_string(),
                                    stream_title: "Synthetic test stream".to_string(),
                                    stream_resolution: "1280 × 720".to_string(),
                                    local_server_online: true,
                                    ..ViewState::default()
                                })
                                .expect("First decoded frame should open the streaming view");
                        }
                        GuiIntent::VideoRendererFailed(_) => event_renderer_failed.set(true),
                        _ => {}
                    }
                }
            },
        );

        let test_publisher = publisher.clone();
        let test_thread = std::thread::Builder::new()
            .name("rabbit-client-video-test".to_string())
            .spawn(move || {
                let runtime = compio::runtime::Runtime::new()
                    .expect("Compio client video test runtime should start");
                let decoded_frames =
                    runtime.block_on(run_test_stream(test_publisher.clone(), seconds));
                test_publisher
                    .quit()
                    .expect("Client video test should stop the Slint event loop");
                decoded_frames
            })
            .expect("Client video test thread should start");

        slint::Timer::single_shot(Duration::from_secs(u64::from(seconds) + 10), || {
            slint::quit_event_loop().expect("Client video test timeout should stop Slint");
        });
        gui.run().expect("Slint client video test should run");
        let decoded_frames = test_thread
            .join()
            .expect("Client video test thread should not panic");

        assert!(
            decoded_frames > 0,
            "Hardware decoder should produce at least one DMA-BUF frame"
        );
        assert!(
            first_frame_presented.get(),
            "The first decoded DMA-BUF should open the streaming view"
        );
        assert!(
            !renderer_failed.get(),
            "Slint bridge should not report a renderer failure"
        );
    }

    async fn run_test_stream(
        publisher: ViewPublisher<<TestApplicationStack as ApplicationStack>::RemoteVideoViewStack>,
        seconds: u32,
    ) -> usize {
        let (pipeline, output) = create_test_rtp_source(seconds);
        pipeline
            .set_state(gstreamer::State::Playing)
            .expect("Synthetic RTP source should start");
        let inputs = TestRtpInputs {
            output,
            screen_id: ScreenId(0),
        };
        let decoded_frames = Rc::new(Cell::new(0_usize));
        let callback_frames = Rc::clone(&decoded_frames);
        let result = <TestApplicationStack as ApplicationStack>::RemoteVideo::run_decoder(
            inputs,
            move |frame| {
                callback_frames.set(callback_frames.get() + 1);
                std::future::ready(publisher.present_video(SessionId(0), ScreenId(0), frame))
            },
            true,
        )
        .await;
        pipeline
            .set_state(gstreamer::State::Null)
            .expect("Synthetic RTP source should stop");
        result.expect("Client video decode and render chain should complete");
        decoded_frames.get()
    }

    struct TestRtpInputs {
        output: gstreamer_app::app_sink::AppSinkStream,
        screen_id: ScreenId,
    }

    impl futures_core::Stream for TestRtpInputs {
        type Item = eros::Result<ReceivedVideoFrame>;

        fn poll_next(
            mut self: Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            match Pin::new(&mut self.output).poll_next(context) {
                std::task::Poll::Ready(Some(sample)) => {
                    std::task::Poll::Ready(Some(sample_to_video_input(self.screen_id, sample)))
                }
                std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        }
    }

    fn sample_to_video_input(
        screen_id: ScreenId,
        sample: gstreamer::Sample,
    ) -> eros::Result<ReceivedVideoFrame> {
        let buffer = sample
            .buffer_owned()
            .expect("Synthetic RTP sample should contain a buffer");
        let buffer = buffer
            .into_mapped_buffer_readable()
            .expect("Synthetic RTP packet should be readable");
        Ok(ReceivedVideoFrame {
            screen_id,
            packets: vec![bytes::Bytes::from_owner(buffer)],
        })
    }

    fn create_test_rtp_source(
        seconds: u32,
    ) -> (gstreamer::Pipeline, gstreamer_app::app_sink::AppSinkStream) {
        gstreamer::init().expect("GStreamer should initialize for the Slint video test");
        let source = test_element("videotestsrc", "test-video-source");
        source.set_property("is-live", true);
        source.set_property(
            "num-buffers",
            i32::try_from(seconds.saturating_mul(30))
                .expect("Client video test duration should fit GStreamer num-buffers"),
        );
        let filter = test_element("capsfilter", "test-video-caps");
        filter.set_property(
            "caps",
            gstreamer::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", 1_280_i32)
                .field("height", 720_i32)
                .field("framerate", gstreamer::Fraction::new(30, 1))
                .build(),
        );
        let encoder = test_element("openh264enc", "test-h264-encoder");
        let parser = test_element("h264parse", "test-h264-parser");
        let payloader = test_element("rtph264pay", "test-rtp-payloader");
        payloader.set_property("mtu", 1_200_u32);
        let sink = test_element("appsink", "test-rtp-output");
        let sink = sink
            .downcast::<gstreamer_app::AppSink>()
            .expect("GStreamer appsink factory should return AppSink");
        sink.set_sync(false);
        sink.set_async(false);
        sink.set_max_buffers(0);
        sink.set_drop(false);
        let output = sink.stream();

        let pipeline = gstreamer::Pipeline::new();
        let elements = [
            &source,
            &filter,
            &encoder,
            &parser,
            &payloader,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(elements)
            .expect("Synthetic RTP source elements should join one pipeline");
        gstreamer::Element::link_many(elements).expect("Synthetic RTP source elements should link");
        (pipeline, output)
    }

    fn test_element(factory: &str, name: &str) -> gstreamer::Element {
        gstreamer::ElementFactory::make(factory)
            .name(name)
            .build()
            .expect("Required synthetic test GStreamer element should be installed")
    }
}
