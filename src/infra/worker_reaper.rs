use std::{
    io,
    thread::{self, JoinHandle},
};

use flume::{Receiver, Sender, unbounded};

enum WorkerReaperCommand {
    Reap(JoinHandle<()>),
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerReaperHandle {
    commands: Sender<WorkerReaperCommand>,
}

#[derive(Debug)]
pub(crate) struct WorkerReaper {
    commands: Option<Sender<WorkerReaperCommand>>,
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
                commands: Some(commands.clone()),
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

        drop(self.commands.take());
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::infra::WorkerReaper;

    #[test]
    fn drains_queued_workers_before_shutdown() {
        let (reaper, handle) = WorkerReaper::new().expect("Test worker reaper should start");
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = Arc::clone(&completed);
        let worker = std::thread::spawn(move || {
            worker_completed.store(true, Ordering::Release);
        });

        handle.reap(worker);
        drop(handle);
        drop(reaper);

        assert!(completed.load(Ordering::Acquire));
    }
}
