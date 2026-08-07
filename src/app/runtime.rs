use std::{
    future::Future,
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
};

use eros::Context;

const APP_COMMAND_CAPACITY: usize = 1;

enum AppCommand {
    Shutdown,
}

pub(crate) trait AppActor {
    fn shutdown(self) -> impl Future<Output = eros::Result<()>>;
}

pub(super) struct AppRuntime;

pub(super) struct AppHandle {
    command_sender: flume::Sender<AppCommand>,
    app_thread: JoinHandle<eros::Result<()>>,
}

impl AppRuntime {
    pub(super) fn start<App>(
        app_constructor: impl FnOnce() -> eros::Result<App> + Send + 'static,
    ) -> eros::Result<AppHandle>
    where
        App: AppActor + 'static,
    {
        let (command_sender, command_receiver) = flume::bounded(APP_COMMAND_CAPACITY);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || run_app_thread(app_constructor, command_receiver, started_sender))
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
    command_receiver: flume::Receiver<AppCommand>,
    started_sender: SyncSender<()>,
) -> eros::Result<()>
where
    App: AppActor + 'static,
{
    let runtime = compio::runtime::Runtime::new()
        .with_context(|| "Failed to create Compio runtime for app")?;
    let app = app_constructor().with_context(|| "Failed to create app container")?;

    started_sender
        .send(())
        .with_context(|| "Failed to report app startup")?;

    runtime.block_on(run_app_actor(app, command_receiver))
}

async fn run_app_actor<App>(
    app: App,
    command_receiver: flume::Receiver<AppCommand>,
) -> eros::Result<()>
where
    App: AppActor,
{
    match command_receiver.recv_async().await {
        Ok(AppCommand::Shutdown) | Err(_) => {}
    }

    app.shutdown().await
}

fn join_app_thread(app_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match app_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("App thread panicked"),
    }
}
