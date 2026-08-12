use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub(crate) struct ResourceUsage {
    used: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ResourceUsageSnapshot {
    pub(crate) used: usize,
    pub(crate) total: usize,
}

impl ResourceUsage {
    pub(crate) fn new(used: usize, total: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(used)),
            total: Arc::new(AtomicUsize::new(total)),
        }
    }

    pub(crate) fn set_used(&self, used: usize) {
        self.used.store(used, Ordering::Relaxed);
    }

    pub(crate) fn set_total(&self, total: usize) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub(crate) fn increment_used(&self) {
        self.used.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn decrement_used(&self) {
        let previous = self.used.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(
            previous > 0,
            "resource usage must be positive before decrement"
        );
    }

    pub(crate) fn snapshot(&self) -> ResourceUsageSnapshot {
        ResourceUsageSnapshot {
            used: self.used.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        }
    }
}
