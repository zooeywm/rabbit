use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MetricsTarget {
    CaptureSource(CaptureSourceId),
    Stream {
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    },
}

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

pub(crate) trait MetricsRecorder {
    fn register_metrics_target(&self);

    fn unregister_metrics_target(&self);

    fn register_capture_pool_usage(&self, usage: ResourceUsage);

    /// Records completion of one source capture frame.
    fn record_captured_frame(&self, frame_id: FrameId, duration: Duration);

    /// Records conversion completion for a source capture frame.
    fn record_converted_frame(&self, frame_id: FrameId, duration: Duration);

    /// Records encoding completion for a source capture frame.
    fn record_encoded_frame(&self, frame_id: FrameId, duration: Duration);
}

pub(crate) trait TransporterMetricsRecorder {
    fn register_transporter_queue_usage(&self, usage: ResourceUsage);

    fn unregister_transporter_queue_usage(&self);

    /// Records packetization completion for a source capture frame.
    fn record_packetized_frame(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        frame_id: FrameId,
        duration: Duration,
    );

    fn record_sent_bytes(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    );
}
