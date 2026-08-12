use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        packetization::outbound_port::Packetizer,
        root::{
            AppContainer, CapturedFrameFor, EncodedBufferFor, EncoderInputFor, PacketizerFor,
            StreamPipelineFor,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                MetricsRecorder, PacketizerManager, PacketizerManagerStateSpec,
            },
        },
        stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

#[cfg(feature = "test-ui")]
pub(crate) mod capture_only;

pub(crate) enum AppMessage {
    #[cfg(feature = "test-ui")]
    CaptureOnly(capture_only::CaptureOnlyMessage),
    StartStream {
        capture_source_id: CaptureSourceId,
        response_sender: flume::Sender<eros::Result<StreamId>>,
    },
    RemoveStream {
        stream_id: StreamId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    CaptureWorkerExited {
        capture_source_id: CaptureSourceId,
    },
    StreamPipelineWorkerExited {
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    },
    Shutdown,
}

pub(super) struct AppRuntime {
    app_handle: AppHandle,
    app_thread: JoinHandle<eros::Result<()>>,
}

#[derive(Clone)]
pub(crate) struct AppHandle {
    message_sender: flume::Sender<AppMessage>,
}

impl AppRuntime {
    pub(super) fn start<CapMgrSt, CvtMgrSt, EcdMgrSt, PktMgrSt, AppRuntimeGuard>(
        app_constructor: impl FnOnce() -> eros::Result<(
            AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, PktMgrSt>,
            AppRuntimeGuard,
        )> + Send
        + 'static,
    ) -> eros::Result<Self>
    where
        CapMgrSt: CapturerManagerStateSpec,
        CvtMgrSt: ConverterManagerStateSpec,
        EcdMgrSt: EncoderManagerStateSpec,
        PktMgrSt: PacketizerManagerStateSpec,
        AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, PktMgrSt>: CapturerManager<State = CapMgrSt>
            + ConverterManager<State = CvtMgrSt>
            + EncoderManager<State = EcdMgrSt>
            + PacketizerManager<State = PktMgrSt>,
        StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
            + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
            + MetricsRecorder,
        EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
        PacketizerFor<PktMgrSt>:
            Packetizer<EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>> + MetricsRecorder,
    {
        let (message_sender, message_receiver) = flume::unbounded();
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let app_message_sender = message_sender.clone();

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || {
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for app")?;
                let (app, _app_runtime_guard) = runtime
                    .enter(app_constructor)
                    .with_context(|| "Failed to construct app")?;

                started_sender
                    .send(())
                    .with_context(|| "Failed to report app startup")?;

                runtime.block_on(app.run(app_message_sender, message_receiver))
            })
            .with_context(|| "Failed to spawn app thread")?;

        if started_receiver.recv().is_err() {
            join_app_thread(app_thread)?;
            eros::bail!("App thread stopped before startup completed");
        }

        Ok(Self {
            app_handle: AppHandle { message_sender },
            app_thread,
        })
    }

    pub(super) fn shutdown(self) -> eros::Result<()> {
        let Self {
            app_handle,
            app_thread,
        } = self;

        let send_result = app_handle.message_sender.send(AppMessage::Shutdown);

        join_app_thread(app_thread)?;

        send_result.with_context(|| "App stopped before receiving shutdown")?;

        Ok(())
    }

    pub(super) fn handle(&self) -> AppHandle {
        self.app_handle.clone()
    }
}

impl AppHandle {
    pub(crate) async fn start_stream(
        &self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<StreamId> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::StartStream {
                capture_source_id,
                response_sender,
            })
            .with_context(|| "App stopped before stream could be started")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while starting stream")?
    }

    pub(crate) async fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::RemoveStream {
                stream_id,
                response_sender,
            })
            .with_context(|| "App stopped before stream could be removed")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while removing stream")?
    }
}

fn join_app_thread(app_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match app_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("App thread panicked"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use super::*;
    use crate::app::container::{
        packetization::{PacketizerContainer, outbound_port::Packetizer},
        screen_capture::outbound_port::{CaptureLoopAction, ScreenCapturer, ScreenCapturerControl},
        stream_pipeline::{StreamPipelineContainer, outbound_port::EncodedVideoFrame},
    };
    use crate::domain::stream::models::vo::FrameId;

    struct TestCapturerManagerState {
        _not_send: Rc<()>,
    }

    struct TestCapturerState;
    struct TestCapturer;
    struct TestControl;
    struct TestConverterManagerState;
    struct TestEncoderManagerState;
    struct TestPacketizerManagerState;

    impl CapturerManagerStateSpec for TestCapturerManagerState {
        type ScreenCapturerState = TestCapturerState;
        type ScreenCapturer = TestCapturer;
    }

    impl From<(CaptureSourceId, TestCapturerState)> for TestCapturer {
        fn from((_capture_source_id, _state): (CaptureSourceId, TestCapturerState)) -> Self {
            Self
        }
    }

    impl MetricsRecorder for TestCapturer {
        fn register_metrics_target(&self) {}

        fn unregister_metrics_target(&self) {}

        fn register_capture_pool_usage(
            &self,
            _usage: crate::app::container::root::outbound_port::ResourceUsage,
        ) {
        }

        fn register_packetizer_queue_usage(
            &self,
            _usage: crate::app::container::root::outbound_port::ResourceUsage,
        ) {
        }

        fn record_captured_frame(&self, _frame_id: FrameId, _duration: std::time::Duration) {}

        fn record_converted_frame(&self, _frame_id: FrameId, _duration: std::time::Duration) {}

        fn record_encoded_frame(&self, _frame_id: FrameId, _duration: std::time::Duration) {}

        fn record_packetized_frame(&self, _frame_id: FrameId, _duration: std::time::Duration) {}
    }

    impl ScreenCapturerControl for TestControl {
        fn wake(&self) -> eros::Result<()> {
            Ok(())
        }
    }

    impl ScreenCapturer for TestCapturer {
        type CapturedFrame = ();
        type Control = TestControl;

        fn control(&self) -> eros::Result<Self::Control> {
            Ok(TestControl)
        }

        fn run<OnStarted, OnControl, OnFrame>(
            &mut self,
            _initial_consumer_count: usize,
            _on_started: OnStarted,
            _on_control: OnControl,
            _on_frame: OnFrame,
        ) -> eros::Result<()>
        where
            OnStarted: FnOnce() -> eros::Result<()>,
            OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
            OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
        {
            unreachable!("the runtime test does not start capture")
        }
    }

    impl ConverterManagerStateSpec for TestConverterManagerState {
        type EncoderFrameConverterState = ();
    }

    impl EncoderManagerStateSpec for TestEncoderManagerState {
        type VideoEncoderState = ();
    }

    impl PacketizerManagerStateSpec for TestPacketizerManagerState {
        type PacketizerState = ();
    }

    type TestApp = AppContainer<
        TestCapturerManagerState,
        TestConverterManagerState,
        TestEncoderManagerState,
        TestPacketizerManagerState,
    >;

    impl CapturerManager for TestApp {
        type State = TestCapturerManagerState;

        fn compose_screen_capturer_state(
            &mut self,
            _capture_source_id: CaptureSourceId,
        ) -> impl FnOnce() -> eros::Result<TestCapturerState> + Send + 'static + use<> {
            || Ok(TestCapturerState)
        }
    }

    impl ConverterManager for TestApp {
        type State = TestConverterManagerState;

        fn compose_encoder_frame_converter_state(
            &mut self,
        ) -> impl FnOnce() -> eros::Result<()> + Send + 'static + use<> {
            || Ok(())
        }
    }

    impl EncoderManager for TestApp {
        type State = TestEncoderManagerState;

        fn compose_video_encoder_state(
            &mut self,
        ) -> impl FnOnce() -> eros::Result<()> + Send + 'static + use<> {
            || Ok(())
        }
    }

    impl PacketizerManager for TestApp {
        type State = TestPacketizerManagerState;

        fn compose_packetizer_state(
            &mut self,
        ) -> impl FnOnce() -> eros::Result<()> + Send + 'static + use<> {
            || Ok(())
        }
    }

    impl EncoderFrameConverter for StreamPipelineContainer<(), ()> {
        type CapturedFrame = ();
        type EncoderInput = ();

        fn convert(&mut self, _frame: ()) -> eros::Result<()> {
            Ok(())
        }
    }

    impl VideoEncoder for StreamPipelineContainer<(), ()> {
        type EncoderInput = ();
        type EncodedBuffer = ();

        fn encode(&mut self, _input: ()) -> eros::Result<EncodedVideoFrame<()>> {
            unreachable!("the runtime test does not encode frames")
        }
    }

    impl Packetizer for PacketizerContainer<()> {
        type EncodedBuffer = ();

        fn packetize(&mut self, _frame: EncodedVideoFrame<()>) -> eros::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn constructs_app_on_app_thread() {
        let caller_thread_id = thread::current().id();
        let created_on_app_thread = Arc::new(AtomicBool::new(false));
        let app_thread_flag = Arc::clone(&created_on_app_thread);

        let app_runtime = AppRuntime::start(move || {
            app_thread_flag.store(
                thread::current().id() != caller_thread_id,
                Ordering::Relaxed,
            );
            Ok((
                TestApp::new(
                    TestCapturerManagerState {
                        _not_send: Rc::new(()),
                    },
                    TestConverterManagerState,
                    TestEncoderManagerState,
                    TestPacketizerManagerState,
                ),
                (),
            ))
        })
        .expect("app runtime should start");

        assert!(created_on_app_thread.load(Ordering::Relaxed));

        app_runtime.shutdown().expect("app should stop cleanly");
    }
}
