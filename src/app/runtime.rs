use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        root::{
            AppContainer, CapturedFrameFor, EncoderInputFor, StreamPipelineFor,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
            },
        },
        stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

pub(crate) enum AppMessage {
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

pub(super) struct AppRuntime;

pub(crate) struct AppHandle {
    message_sender: flume::Sender<AppMessage>,
    app_thread: JoinHandle<eros::Result<()>>,
}

impl AppRuntime {
    pub(super) fn start<CapMgrSt, CvtMgrSt, EcdMgrSt>(
        app_constructor: impl FnOnce() -> eros::Result<AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>>
        + Send
        + 'static,
    ) -> eros::Result<AppHandle>
    where
        CapMgrSt: CapturerManagerStateSpec,
        CvtMgrSt: ConverterManagerStateSpec,
        EcdMgrSt: EncoderManagerStateSpec,
        AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>: CapturerManager<State = CapMgrSt>
            + ConverterManager<State = CvtMgrSt>
            + EncoderManager<State = EcdMgrSt>,
        StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
            + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>,
    {
        let (message_sender, message_receiver) = flume::unbounded();
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let app_message_sender = message_sender.clone();

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || {
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for app")?;
                let app = runtime
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

        Ok(AppHandle {
            message_sender,
            app_thread,
        })
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

    pub(super) fn shutdown(self) -> eros::Result<()> {
        let Self {
            message_sender,
            app_thread,
        } = self;

        let send_result = message_sender.send(AppMessage::Shutdown);

        join_app_thread(app_thread)?;

        send_result.with_context(|| "App stopped before receiving shutdown")?;

        Ok(())
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
        screen_capture::outbound_port::{CaptureLoopAction, ScreenCapturer, ScreenCapturerControl},
        stream_pipeline::{StreamPipelineContainer, outbound_port::EncodedVideoFrame},
    };

    struct TestCapturerManagerState {
        _not_send: Rc<()>,
    }

    struct TestCapturerState;
    struct TestCapturer;
    struct TestControl;
    struct TestConverterManagerState;
    struct TestEncoderManagerState;

    impl CapturerManagerStateSpec for TestCapturerManagerState {
        type ScreenCapturerState = TestCapturerState;
        type ScreenCapturer = TestCapturer;
    }

    impl From<TestCapturerState> for TestCapturer {
        fn from(_state: TestCapturerState) -> Self {
            Self
        }
    }

    impl ScreenCapturerControl for TestControl {
        fn wake(&self) -> eros::Result<()> {
            Ok(())
        }
    }

    impl ScreenCapturer for TestCapturer {
        type CapturedFrame = ();

        fn control(&self) -> eros::Result<Arc<dyn ScreenCapturerControl>> {
            Ok(Arc::new(TestControl))
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

    type TestApp =
        AppContainer<TestCapturerManagerState, TestConverterManagerState, TestEncoderManagerState>;

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

    #[test]
    fn constructs_app_on_app_thread() {
        let caller_thread_id = thread::current().id();
        let created_on_app_thread = Arc::new(AtomicBool::new(false));
        let app_thread_flag = Arc::clone(&created_on_app_thread);

        let app_handle = AppRuntime::start(move || {
            app_thread_flag.store(
                thread::current().id() != caller_thread_id,
                Ordering::Relaxed,
            );
            Ok(TestApp::new(
                TestCapturerManagerState {
                    _not_send: Rc::new(()),
                },
                TestConverterManagerState,
                TestEncoderManagerState,
            ))
        })
        .expect("app runtime should start");

        assert!(created_on_app_thread.load(Ordering::Relaxed));

        app_handle.shutdown().expect("app should stop cleanly");
    }
}
