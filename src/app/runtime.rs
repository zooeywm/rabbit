use std::{
    future::Future,
    sync::{
        Arc, Weak,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

pub(crate) enum AppCommand {
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

pub(crate) trait AppActor {
    fn run(
        self,
        command_sender: Weak<flume::Sender<AppCommand>>,
        command_receiver: flume::Receiver<AppCommand>,
    ) -> impl Future<Output = eros::Result<()>>;
}

pub(super) struct AppRuntime;

pub(crate) struct AppHandle {
    command_sender: Arc<flume::Sender<AppCommand>>,
    app_thread: JoinHandle<eros::Result<()>>,
}

impl AppRuntime {
    pub(super) fn start<App>(
        app_constructor: impl FnOnce() -> eros::Result<App> + Send + 'static,
    ) -> eros::Result<AppHandle>
    where
        App: AppActor + 'static,
    {
        let (command_sender, command_receiver) = flume::unbounded();
        let command_sender = Arc::new(command_sender);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let actor_command_sender = Arc::downgrade(&command_sender);

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || {
                run_app_thread(
                    app_constructor,
                    actor_command_sender,
                    command_receiver,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn app thread")?;

        if started_receiver.recv().is_err() {
            join_app_thread(app_thread)?;
            eros::bail!("App thread stopped before startup completed");
        }

        Ok(AppHandle {
            command_sender,
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

        self.command_sender
            .send(AppCommand::StartStream {
                capture_source_id,
                response_sender,
            })
            .with_context(|| "App actor stopped before stream could be started")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App actor stopped while starting stream")?
    }

    pub(crate) async fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.command_sender
            .send(AppCommand::RemoveStream {
                stream_id,
                response_sender,
            })
            .with_context(|| "App actor stopped before stream could be removed")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App actor stopped while removing stream")?
    }

    pub(super) fn shutdown(self) -> eros::Result<()> {
        let Self {
            command_sender,
            app_thread,
        } = self;

        let send_result = command_sender.send(AppCommand::Shutdown);

        join_app_thread(app_thread)?;

        send_result.with_context(|| "App actor stopped before receiving shutdown")?;

        Ok(())
    }
}

fn run_app_thread<App>(
    app_constructor: impl FnOnce() -> eros::Result<App>,
    command_sender: Weak<flume::Sender<AppCommand>>,
    command_receiver: flume::Receiver<AppCommand>,
    started_sender: SyncSender<()>,
) -> eros::Result<()>
where
    App: AppActor + 'static,
{
    let runtime = compio::runtime::Runtime::new()
        .with_context(|| "Failed to create Compio runtime for app")?;
    let app = runtime
        .enter(app_constructor)
        .with_context(|| "Failed to construct app")?;

    started_sender
        .send(())
        .with_context(|| "Failed to report app startup")?;

    runtime.block_on(app.run(command_sender, command_receiver))
}

fn join_app_thread(app_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match app_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("App thread panicked"),
    }
}
