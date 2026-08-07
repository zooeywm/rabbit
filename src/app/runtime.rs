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

pub(crate) trait AppActor {
    fn run(
        self,
        message_sender: Weak<flume::Sender<AppMessage>>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> impl Future<Output = eros::Result<()>>;
}

pub(super) struct AppRuntime;

pub(crate) struct AppHandle {
    message_sender: Arc<flume::Sender<AppMessage>>,
    app_thread: JoinHandle<eros::Result<()>>,
}

impl AppRuntime {
    pub(super) fn start<App>(
        app_constructor: impl FnOnce() -> eros::Result<App> + Send + 'static,
    ) -> eros::Result<AppHandle>
    where
        App: AppActor + 'static,
    {
        let (message_sender, message_receiver) = flume::unbounded();
        let message_sender = Arc::new(message_sender);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let actor_message_sender = Arc::downgrade(&message_sender);

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || {
                run_app_thread(
                    app_constructor,
                    actor_message_sender,
                    message_receiver,
                    started_sender,
                )
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
            .with_context(|| "App actor stopped before stream could be started")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App actor stopped while starting stream")?
    }

    pub(crate) async fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::RemoveStream {
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
            message_sender,
            app_thread,
        } = self;

        let send_result = message_sender.send(AppMessage::Shutdown);

        join_app_thread(app_thread)?;

        send_result.with_context(|| "App actor stopped before receiving shutdown")?;

        Ok(())
    }
}

fn run_app_thread<App>(
    app_constructor: impl FnOnce() -> eros::Result<App>,
    message_sender: Weak<flume::Sender<AppMessage>>,
    message_receiver: flume::Receiver<AppMessage>,
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

    runtime.block_on(app.run(message_sender, message_receiver))
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

    struct NonSendApp {
        _not_send: Rc<()>,
    }

    impl AppActor for NonSendApp {
        async fn run(
            self,
            _message_sender: Weak<flume::Sender<AppMessage>>,
            message_receiver: flume::Receiver<AppMessage>,
        ) -> eros::Result<()> {
            match message_receiver.recv_async().await {
                Ok(AppMessage::Shutdown) | Err(_) => Ok(()),
                Ok(_) => eros::bail!("NonSendApp received an unexpected command"),
            }
        }
    }

    #[test]
    fn constructs_non_send_app_on_app_thread() {
        let caller_thread_id = thread::current().id();
        let created_on_app_thread = Arc::new(AtomicBool::new(false));
        let app_thread_flag = Arc::clone(&created_on_app_thread);

        let app_handle = AppRuntime::start(move || {
            app_thread_flag.store(
                thread::current().id() != caller_thread_id,
                Ordering::Relaxed,
            );

            Ok(NonSendApp {
                _not_send: Rc::new(()),
            })
        })
        .expect("app runtime should accept a non-Send app");

        assert!(created_on_app_thread.load(Ordering::Relaxed));

        app_handle.shutdown().expect("app should stop cleanly");
    }
}
