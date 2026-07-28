#[derive(Clone, Debug)]
pub(crate) struct WorkerReaperHandle;

#[derive(Debug)]
pub(crate) struct WorkerReaper;

impl WorkerReaper {
    pub(crate) fn new() -> std::io::Result<(Self, WorkerReaperHandle)> {
        Ok((Self, WorkerReaperHandle))
    }
}
