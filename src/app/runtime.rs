use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        network::{NetworkContainer, inbound::NetworkWorker, outbound_port::Transporter},
        root::{
            AppContainer, CapturedFrameFor, EncodedBufferFor, EncoderInputFor, StreamPipelineFor,
            TransporterStateFor,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                MetricsRecorder, NetworkMetricsRecorder, TransporterConstructor,
                TransporterConstructorStateSpec,
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
    NetworkWorkerExited,
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
    pub(super) fn start<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt, AppRuntimeGuard>(
        app_constructor: impl FnOnce() -> eros::Result<(
            AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>,
            AppRuntimeGuard,
        )> + Send
        + 'static,
    ) -> eros::Result<Self>
    where
        CapMgrSt: CapturerManagerStateSpec,
        CvtMgrSt: ConverterManagerStateSpec,
        EcdMgrSt: EncoderManagerStateSpec,
        TprCstSt: TransporterConstructorStateSpec,
        NetworkContainer<TransporterStateFor<TprCstSt>>: Transporter<EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>>
            + NetworkMetricsRecorder,
        AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>: CapturerManager<State = CapMgrSt>
            + ConverterManager<State = CvtMgrSt>
            + EncoderManager<State = EcdMgrSt>
            + TransporterConstructor<State = TprCstSt>,
        StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
            + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
            + MetricsRecorder,
        EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
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
                let transporter_constructor = app.compose_transporter()?;
                let network_worker =
                    NetworkWorker::spawn(transporter_constructor, app_message_sender.clone())?;
                let encoded_unit_sender = network_worker.sender();

                started_sender
                    .send(())
                    .with_context(|| "Failed to report app startup")?;

                let app_result = runtime.block_on(app.run(
                    encoded_unit_sender,
                    app_message_sender,
                    message_receiver,
                ));
                let network_result = runtime.block_on(network_worker.shutdown());

                match network_result {
                    Err(error) => Err(error),
                    Ok(()) => app_result,
                }
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
