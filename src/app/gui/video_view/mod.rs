use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use eros::Context as _;
use slint::{ComponentHandle as _, RenderingState};

use crate::{
    app::gui::view::{GuiIntent, RabbitWindow},
    kernel::{
        screen_manager::ScreenId, session::SessionId,
        video_decoder::DecodedVideoFrame as DecodedVideoFrameTrait,
    },
};

pub(crate) trait VideoViewStack: 'static {
    type Frame: DecodedVideoFrameTrait + 'static;
    type NativeRenderer: 'static;
    type NativeViewport;

    fn select_slint_backend() -> eros::Result<()>;

    fn create_native_renderer(
        window: &slint::Window,
        probe_interval: Duration,
    ) -> eros::Result<Self::NativeRenderer>;

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
    display: Option<Box<Stack::NativeRenderer>>,
    active_stream: Option<(SessionId, ScreenId)>,
    failed: bool,
}

pub(crate) fn install<Stack>(
    window: &RabbitWindow,
    errors: flume::Sender<GuiIntent>,
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
    let view_state = Rc::new(RefCell::new(VideoViewState::<Stack> {
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
        let VideoViewState {
            display,
            active_stream,
            ..
        } = &mut *state;
        let result = if display.is_some() {
            render_native_frame(
                direct_commands.clone(),
                &direct_window,
                display,
                active_stream,
            )
        } else {
            if let Some(window) = direct_window.upgrade() {
                window.window().request_redraw();
            }
            Ok(None)
        };
        match result {
            Ok(Some((session_id, screen_id))) => {
                let _ = direct_errors.send(GuiIntent::VideoFrameReady {
                    session_id,
                    screen_id,
                });
            }
            Ok(None) => {}
            Err(error) => fail_video_display(&mut state, direct_errors.clone(), error),
        }
    });

    let rendering_state = Rc::clone(&view_state);
    window
        .window()
        .set_rendering_notifier(move |state, _graphics_api| {
            let mut video = rendering_state.borrow_mut();
            if video.failed {
                return;
            }
            let result = match state {
                RenderingState::RenderingSetup => Ok(()),
                RenderingState::AfterRendering => {
                    let VideoViewState {
                        display,
                        active_stream,
                        ..
                    } = &mut *video;
                    let result = render_video_frame(
                        receiver.clone(),
                        &weak_window,
                        display,
                        active_stream,
                        probe_interval,
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
                    Some(mut display) => Stack::teardown_native_renderer(&mut display),
                    None => Ok(()),
                },
                RenderingState::BeforeRendering => Ok(()),
                _ => Ok(()),
            };

            if let Err(error) = result {
                fail_video_display(&mut video, errors.clone(), error);
            }
        })
        .context("Failed to install the native video surface rendering bridge")?;

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
    commands: flume::Receiver<VideoViewCommand<Stack>>,
    weak_window: &slint::Weak<RabbitWindow>,
    display: &mut Option<Box<Stack::NativeRenderer>>,
    active_stream: &mut Option<(SessionId, ScreenId)>,
    probe_interval: Duration,
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
                    *display = Some(Box::new(Stack::create_native_renderer(
                        window.window(),
                        probe_interval,
                    )?));
                }
                let renderer = display
                    .as_mut()
                    .with_context(|| "Native video display disappeared during initialization")?;
                Stack::validate_native_frame(renderer, &frame)?;
                Stack::present_native_frame(renderer, *frame);
                if initialized_display {
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
                    Stack::clear_native_renderer(display)?;
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
        Stack::set_native_viewport(display, Stack::native_viewport(&window, false)?)?;
        Stack::render_native_renderer(display)?;
        return Ok(presented);
    }
    Stack::set_native_viewport(display, Stack::native_viewport(&window, true)?)?;
    Stack::render_native_renderer(display)?;
    Ok(presented)
}

fn render_native_frame<Stack>(
    commands: flume::Receiver<VideoViewCommand<Stack>>,
    weak_window: &slint::Weak<RabbitWindow>,
    display: &mut Option<Box<Stack::NativeRenderer>>,
    active_stream: &mut Option<(SessionId, ScreenId)>,
) -> eros::Result<Option<(SessionId, ScreenId)>>
where
    Stack: VideoViewStack,
{
    let Some(renderer) = display.as_mut() else {
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

fn activate_stream(
    active_stream: &mut Option<(SessionId, ScreenId)>,
    session_id: SessionId,
    screen_id: ScreenId,
) -> bool {
    let first_frame = *active_stream != Some((session_id, screen_id));
    *active_stream = Some((session_id, screen_id));
    first_frame
}

fn report_error_once(errors: flume::Sender<GuiIntent>, failed: &mut bool, error: String) {
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
    errors: flume::Sender<GuiIntent>,
    error: eros::ErrorUnion,
) where
    Stack: VideoViewStack,
{
    let cleanup_error = state
        .display
        .as_mut()
        .and_then(|display| Stack::teardown_native_renderer(display).err());
    state.display = None;
    let mut error = format!("{error:?}");
    if let Some(cleanup_error) = cleanup_error {
        error.push_str(&format!(
            "\nAdditionally failed to release video renderer resources: {cleanup_error:?}"
        ));
    }
    report_error_once(errors, &mut state.failed, error);
}

#[cfg(test)]
mod tests {
    use crate::kernel::{screen_manager::ScreenId, session::SessionId};

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
}
