use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        client::inbound_port::ClientApplication,
        host::{inbound_port::HostApplication, outbound_port::HostEventReporter},
        network::{
            NetworkContainer,
            inbound::NetworkWorker,
            outbound_port::{NetworkMetricsRecorder, TransporterClientSide, TransporterHostSide},
        },
        root::{
            AppContainer, TransporterStateFor,
            outbound_port::{TransporterConstructor, TransporterConstructorStateSpec},
        },
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
    HostStreamPipelineWorkerExited {
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    },
    NetworkWorkerExited,
    Shutdown,
}

impl HostEventReporter for flume::Sender<AppMessage> {
    fn report_capture_worker_exited(&self, capture_source_id: CaptureSourceId) {
        let _ = self.send(AppMessage::CaptureWorkerExited { capture_source_id });
    }

    fn report_host_stream_pipeline_worker_exited(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) {
        let _ = self.send(AppMessage::HostStreamPipelineWorkerExited {
            capture_source_id,
            stream_id,
        });
    }
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
    pub(super) fn start<Host, Client, NetworkConstructorState, AppRuntimeGuard>(
        app_constructor: impl FnOnce() -> eros::Result<(
            AppContainer<Host, Client, NetworkConstructorState>,
            AppRuntimeGuard,
        )> + Send
        + 'static,
    ) -> eros::Result<Self>
    where
        Host: HostApplication,
        Client: ClientApplication,
        NetworkConstructorState: TransporterConstructorStateSpec,
        NetworkContainer<TransporterStateFor<NetworkConstructorState>>: TransporterHostSide<EncodedBuffer = Host::EncodedBuffer>
            + TransporterClientSide<Depacketized = Client::NetworkInput>
            + NetworkMetricsRecorder,
        AppContainer<Host, Client, NetworkConstructorState>:
            TransporterConstructor<State = NetworkConstructorState>,
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
                let mut network_worker =
                    NetworkWorker::spawn(transporter_constructor, app_message_sender.clone())?;
                let encoded_unit_sender = network_worker.sender();
                let network_client_event_receiver = network_worker.take_client_event_receiver()?;

                started_sender
                    .send(())
                    .with_context(|| "Failed to report app startup")?;

                let app_result = runtime.block_on(app.run(
                    encoded_unit_sender,
                    network_client_event_receiver,
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
