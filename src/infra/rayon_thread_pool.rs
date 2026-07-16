use eros::Context;

#[derive(Debug, kudi::DepInj)]
#[target(RayonThreadPool)]
pub(crate) struct RayonThreadPoolState {
    thread_pool: rayon::ThreadPool,
}

impl RayonThreadPoolState {
    pub(crate) fn new() -> eros::Result<Self> {
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .build()
            .context("Failed to create the Rayon thread pool")?;

        Ok(Self { thread_pool })
    }

    pub(crate) fn spawn<Task>(&self, task: Task)
    where
        Task: FnOnce() + Send + 'static,
    {
        self.thread_pool.spawn(task);
    }
}
