use std::{
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
};

use eros::Context;

const ROOT_COMMAND_CAPACITY: usize = 1;

enum RootCommand {
    Shutdown,
}

pub(crate) trait RootActor {
    async fn shutdown(self) -> eros::Result<()>;
}

pub(super) struct RootHandle {
    command_sender: flume::Sender<RootCommand>,
    root_thread: JoinHandle<eros::Result<()>>,
}

impl RootHandle {
    pub(super) fn start<Root>(
        create_root: impl FnOnce() -> eros::Result<Root> + Send + 'static,
    ) -> eros::Result<Self>
    where
        Root: RootActor + 'static,
    {
        let (command_sender, command_receiver) = flume::bounded(ROOT_COMMAND_CAPACITY);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);

        let root_thread = thread::Builder::new()
            .name("root".to_owned())
            .spawn(move || run_root_thread(create_root, command_receiver, started_sender))
            .with_context(|| "Failed to spawn root thread")?;

        if started_receiver.recv().is_err() {
            join_root_thread(root_thread)?;
            eros::bail!("Root thread stopped before startup completed");
        }

        Ok(Self {
            command_sender,
            root_thread,
        })
    }

    pub(super) fn shutdown(self) -> eros::Result<()> {
        let Self {
            command_sender,
            root_thread,
        } = self;

        let send_result = command_sender.send(RootCommand::Shutdown);

        join_root_thread(root_thread)?;

        send_result.with_context(|| "Root actor stopped before receiving shutdown")?;

        Ok(())
    }
}

fn run_root_thread<Root>(
    create_root: impl FnOnce() -> eros::Result<Root>,
    command_receiver: flume::Receiver<RootCommand>,
    started_sender: SyncSender<()>,
) -> eros::Result<()>
where
    Root: RootActor + 'static,
{
    let runtime = compio::runtime::Runtime::new()
        .with_context(|| "Failed to create Compio runtime for root")?;
    let root = create_root().with_context(|| "Failed to create root container")?;

    started_sender
        .send(())
        .with_context(|| "Failed to report root startup")?;

    runtime.block_on(run_root_actor(root, command_receiver))
}

async fn run_root_actor<Root>(
    root: Root,
    command_receiver: flume::Receiver<RootCommand>,
) -> eros::Result<()>
where
    Root: RootActor,
{
    match command_receiver.recv_async().await {
        Ok(RootCommand::Shutdown) | Err(_) => {}
    }

    root.shutdown().await
}

fn join_root_thread(root_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match root_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Root thread panicked"),
    }
}
