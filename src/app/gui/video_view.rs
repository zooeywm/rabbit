use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use eros::Context as _;
use slint::{ComponentHandle as _, GraphicsAPI, RenderingState};

use crate::{
    app::gui::view::{GuiIntent, RabbitWindow},
    infra::{GStreamerDecodedFrame, OpenGlVideoRenderer},
    kernel::{
        screen_manager::ScreenId,
        session::SessionId,
        video_renderer::{VideoRenderer as _, VideoViewport},
    },
};

enum VideoViewCommand {
    Present {
        session_id: SessionId,
        screen_id: ScreenId,
        frame: GStreamerDecodedFrame,
    },
    Clear,
}

#[derive(Clone)]
pub(crate) struct VideoViewPublisher {
    sender: flume::Sender<VideoViewCommand>,
    stale: flume::Receiver<VideoViewCommand>,
    window: slint::Weak<RabbitWindow>,
    redraw_scheduled: Arc<AtomicBool>,
}

pub(crate) fn install(
    window: &RabbitWindow,
    errors: flume::Sender<GuiIntent>,
) -> eros::Result<VideoViewPublisher> {
    let (sender, receiver) = flume::bounded(1);
    let publisher = VideoViewPublisher {
        sender,
        stale: receiver.clone(),
        window: window.as_weak(),
        redraw_scheduled: Arc::new(AtomicBool::new(false)),
    };
    let weak_window = window.as_weak();
    let mut renderer = None;
    let mut active_stream = None;
    let mut failed = false;
    let mut initialization_error = None;

    window
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            let result = match state {
                RenderingState::RenderingSetup => {
                    let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
                        return report_error_once(
                            &errors,
                            &mut failed,
                            "Slint did not provide the required native OpenGL renderer".to_string(),
                        );
                    };
                    match OpenGlVideoRenderer::new(*get_proc_address) {
                        Ok(created) => {
                            renderer = Some(created);
                            Ok(())
                        }
                        Err(error) => {
                            initialization_error = Some(format!("{error:?}"));
                            Ok(())
                        }
                    }
                }
                RenderingState::AfterRendering => {
                    let result = render_video_frame(
                        &receiver,
                        &weak_window,
                        &mut renderer,
                        &mut active_stream,
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
                RenderingState::RenderingTeardown => match renderer.take() {
                    Some(mut renderer) => renderer.teardown(),
                    None => Ok(()),
                },
                RenderingState::BeforeRendering => Ok(()),
                _ => Ok(()),
            };

            if let Err(error) = result {
                let cleanup_error = renderer
                    .as_mut()
                    .and_then(|renderer| renderer.teardown().err());
                renderer = None;
                let mut error = initialization_error
                    .take()
                    .unwrap_or_else(|| format!("{error:?}"));
                if let Some(cleanup_error) = cleanup_error {
                    error.push_str(&format!(
                        "\nAdditionally failed to release video renderer resources: {cleanup_error:?}"
                    ));
                }
                report_error_once(&errors, &mut failed, error);
            }
        })
        .context("Failed to install the Slint DMA-BUF video rendering bridge")?;

    Ok(publisher)
}

impl VideoViewPublisher {
    pub(crate) fn present(
        &self,
        session_id: SessionId,
        screen_id: ScreenId,
        frame: GStreamerDecodedFrame,
    ) -> eros::Result<()> {
        if frame.screen_id != screen_id {
            eros::bail!(
                "Decoded screen {} frame cannot be presented for screen {}",
                frame.screen_id.0,
                screen_id.0
            );
        }
        self.publish(VideoViewCommand::Present {
            session_id,
            screen_id,
            frame,
        })
    }

    pub(crate) fn clear(&self) -> eros::Result<()> {
        self.publish(VideoViewCommand::Clear)
    }

    fn publish(&self, mut command: VideoViewCommand) -> eros::Result<()> {
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

        if self.redraw_scheduled.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let window = self.window.clone();
        let redraw_scheduled = Arc::clone(&self.redraw_scheduled);
        if let Err(error) = slint::invoke_from_event_loop(move || {
            redraw_scheduled.store(false, Ordering::Relaxed);
            if let Some(window) = window.upgrade() {
                window.window().request_redraw();
            }
        }) {
            self.redraw_scheduled.store(false, Ordering::Relaxed);
            Err::<(), _>(error)
                .context("Failed to request a Slint redraw for a decoded video frame")?;
        }
        Ok(())
    }
}

fn render_video_frame(
    commands: &flume::Receiver<VideoViewCommand>,
    weak_window: &slint::Weak<RabbitWindow>,
    renderer: &mut Option<OpenGlVideoRenderer>,
    active_stream: &mut Option<(SessionId, ScreenId)>,
) -> eros::Result<Option<(SessionId, ScreenId)>> {
    let mut presented = None;
    if let Ok(command) = commands.try_recv() {
        match command {
            VideoViewCommand::Present {
                session_id,
                screen_id,
                frame,
            } => {
                let renderer = renderer.as_mut().context(
                    "Slint requested video rendering before the OpenGL renderer was ready",
                )?;
                *active_stream = Some((session_id, screen_id));
                renderer.present(frame);
                presented = Some((session_id, screen_id));
            }
            VideoViewCommand::Clear => {
                *active_stream = None;
                if let Some(renderer) = renderer.as_mut() {
                    renderer.clear()?;
                }
            }
        }
    }

    let Some(window) = weak_window.upgrade() else {
        return Ok(None);
    };
    let Some(renderer) = renderer.as_mut() else {
        return Ok(None);
    };
    if !window.get_video_viewport_visible() || active_stream.is_none() {
        return Ok(presented);
    }
    let scale = window.window().scale_factor();
    renderer.set_viewport(VideoViewport {
        x: physical_pixels(window.get_video_viewport_x(), scale)?,
        y: physical_pixels(window.get_video_viewport_y(), scale)?,
        width: physical_pixels(window.get_video_viewport_width(), scale)?,
        height: physical_pixels(window.get_video_viewport_height(), scale)?,
    });
    renderer.render()?;
    Ok(presented)
}

fn physical_pixels(logical: f32, scale: f32) -> eros::Result<u32> {
    let physical = logical * scale;
    if !physical.is_finite() || physical < 0.0 || physical > u32::MAX as f32 {
        eros::bail!("Invalid physical video viewport coordinate {}", physical);
    }
    Ok(physical.round() as u32)
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

// Focused hardware test: scripts/test-client-video [positive-seconds]
#[cfg(test)]
mod tests {
    use std::{cell::Cell, pin::Pin, rc::Rc, time::Duration};

    use gstreamer::glib::prelude::{Cast as _, ObjectExt as _};
    use gstreamer::prelude::{ElementExt as _, GstBinExtManual as _};

    use crate::{
        app::gui::{
            state::{ViewPage, ViewState},
            view::{Gui, GuiIntent, ViewPublisher},
        },
        infra::GStreamerVideoDecoder,
        kernel::{
            screen_manager::ScreenId,
            session::{ReceivedVideoFrame, SessionId},
        },
    };

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
            Gui::new().expect("Slint video test window should be created");
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

    async fn run_test_stream(publisher: ViewPublisher, seconds: u32) -> usize {
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
        let result = GStreamerVideoDecoder::run_with_probing(
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
