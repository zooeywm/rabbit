use std::{
    io,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, unbounded};

enum WorkerReaperCommand {
    Reap(JoinHandle<()>),
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerReaperHandle {
    commands: Sender<WorkerReaperCommand>,
}

#[derive(Debug)]
pub(crate) struct WorkerReaper {
    commands: Sender<WorkerReaperCommand>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerReaper {
    pub(crate) fn new() -> io::Result<(Self, WorkerReaperHandle)> {
        let (commands, receiver) = unbounded();
        let thread = thread::Builder::new()
            .name("rabbit-worker-reaper".to_owned())
            .spawn(move || run_worker_reaper(receiver))?;

        Ok((
            Self {
                commands: commands.clone(),
                thread: Some(thread),
            },
            WorkerReaperHandle { commands },
        ))
    }
}

impl WorkerReaperHandle {
    pub(crate) fn reap(&self, thread: JoinHandle<()>) {
        let _ = self.commands.send(WorkerReaperCommand::Reap(thread));
    }
}

impl Drop for WorkerReaper {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };

        let _ = self.commands.send(WorkerReaperCommand::Shutdown);
        let _ = thread.join();
    }
}

fn run_worker_reaper(commands: Receiver<WorkerReaperCommand>) {
    while let Ok(command) = commands.recv() {
        match command {
            WorkerReaperCommand::Reap(thread) => {
                if thread.join().is_err() {
                    tracing::error!("Background worker thread panicked while shutting down");
                }
            }
            WorkerReaperCommand::Shutdown => return,
        }
    }
}
