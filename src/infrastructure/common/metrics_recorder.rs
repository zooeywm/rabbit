use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    sync::{Mutex, OnceLock},
};

use opentelemetry::{
    KeyValue, global,
    metrics::{Histogram, UpDownCounter},
};

use crate::{
    app::container::root::outbound_port::{
        MetricsRecorder, MetricsTarget, NetworkMetricsRecorder, ResourceUsage,
        ResourceUsageSnapshot,
    },
    domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
};

pub(super) const CAPTURE_SOURCE_ID_ATTRIBUTE: &str = "capture_source_id";
pub(super) const STREAM_ID_ATTRIBUTE: &str = "stream_id";
pub(super) const CAPTURE_DURATION_METRIC: &str = "rabbit.capture.duration";
pub(super) const CONVERT_DURATION_METRIC: &str = "rabbit.convert.duration";
pub(super) const ENCODE_DURATION_METRIC: &str = "rabbit.encode.duration";
pub(super) const PACKETIZE_DURATION_METRIC: &str = "rabbit.packetize.duration";

#[derive(kudi::DepInj)]
#[target(OpenTelemetryMetricsRecorderImpl)]
pub(crate) struct OpenTelemetryMetricsRecorder;

#[derive(Default)]
struct CompletedSourceFrameIds {
    captured: HashSet<FrameId>,
    encoded: HashSet<FrameId>,
    packetized: HashSet<FrameId>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct CompletedSourceFrameCounts {
    pub(super) captured: usize,
    pub(super) encoded: usize,
    pub(super) packetized: usize,
}

#[derive(Default)]
struct TargetResourceUsages {
    capture_pool: Option<ResourceUsage>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct TargetResourceUsageSnapshots {
    pub(super) capture_pool: Option<ResourceUsageSnapshot>,
}

#[derive(Clone, Copy)]
enum SourceFrameStage {
    Capture,
    Encode,
    Packetize,
}

struct Instruments {
    active_targets: UpDownCounter<i64>,
    capture_duration: Histogram<f64>,
    convert_duration: Histogram<f64>,
    encode_duration: Histogram<f64>,
    packetize_duration: Histogram<f64>,
}

impl Instruments {
    fn global() -> &'static Self {
        static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();

        INSTRUMENTS.get_or_init(|| {
            let meter = global::meter("rabbit");

            Self {
                active_targets: meter
                    .i64_up_down_counter("rabbit.metrics.active_targets")
                    .with_description("Number of active metrics targets")
                    .build(),
                capture_duration: meter
                    .f64_histogram(CAPTURE_DURATION_METRIC)
                    .with_description("Time spent capturing one frame")
                    .with_unit("ms")
                    .build(),
                convert_duration: meter
                    .f64_histogram(CONVERT_DURATION_METRIC)
                    .with_description("Time spent converting one source frame")
                    .with_unit("ms")
                    .build(),
                encode_duration: meter
                    .f64_histogram(ENCODE_DURATION_METRIC)
                    .with_description("Time spent completing encoding for one source frame")
                    .with_unit("ms")
                    .build(),
                packetize_duration: meter
                    .f64_histogram(PACKETIZE_DURATION_METRIC)
                    .with_description("Time spent completing packetization for one source frame")
                    .with_unit("ms")
                    .build(),
            }
        })
    }
}

impl<Deps> MetricsRecorder for OpenTelemetryMetricsRecorderImpl<Deps>
where
    Deps: AsRef<MetricsTarget>,
{
    fn register_metrics_target(&self) {
        let target = *self.prj_ref().as_ref();
        let should_activate = {
            let mut targets = registered_metrics_targets()
                .lock()
                .expect("metrics target registry mutex should not be poisoned");

            match targets.entry(target) {
                Entry::Vacant(entry) => {
                    entry.insert(1);
                    true
                }
                Entry::Occupied(mut entry) => {
                    let next_count = entry
                        .get()
                        .checked_add(1)
                        .expect("metrics target registration count should not overflow");
                    *entry.get_mut() = next_count;
                    false
                }
            }
        };

        if should_activate {
            clear_completed_source_frames(target);
            clear_metrics_resource_usages(target);
            Instruments::global()
                .active_targets
                .add(1, &target_attributes(target));
        }
    }

    fn unregister_metrics_target(&self) {
        let target = *self.prj_ref().as_ref();
        let should_deactivate = {
            let mut targets = registered_metrics_targets()
                .lock()
                .expect("metrics target registry mutex should not be poisoned");
            let count = targets
                .get_mut(&target)
                .expect("metrics target should be registered before removal");

            let next_count = count
                .checked_sub(1)
                .expect("metrics target registration count should be positive");
            *count = next_count;

            if *count == 0 {
                targets.remove(&target);
                true
            } else {
                false
            }
        };

        if should_deactivate {
            clear_completed_source_frames(target);
            clear_metrics_resource_usages(target);
            Instruments::global()
                .active_targets
                .add(-1, &target_attributes(target));
        }
    }

    fn register_capture_pool_usage(&self, usage: ResourceUsage) {
        let target = *self.prj_ref().as_ref();
        assert!(
            matches!(target, MetricsTarget::CaptureSource(_)),
            "capture pool usage requires a capture source metrics target",
        );

        metrics_resource_usages()
            .lock()
            .expect("metrics resource usage registry mutex should not be poisoned")
            .entry(target)
            .or_default()
            .capture_pool = Some(usage);
    }

    fn record_captured_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        let target = *self.prj_ref().as_ref();
        let MetricsTarget::CaptureSource(capture_source_id) = target else {
            unreachable!("captured frames require a capture source metrics target");
        };
        record_completed_source_frame(target, SourceFrameStage::Capture, frame_id);

        Instruments::global().capture_duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[KeyValue::new(
                CAPTURE_SOURCE_ID_ATTRIBUTE,
                i64::from(capture_source_id.value()),
            )],
        );
    }

    fn record_converted_frame(&self, _frame_id: FrameId, duration: std::time::Duration) {
        let target = *self.prj_ref().as_ref();
        let MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        } = target
        else {
            unreachable!("converted frames require a stream metrics target");
        };
        Instruments::global().convert_duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[
                KeyValue::new(
                    CAPTURE_SOURCE_ID_ATTRIBUTE,
                    i64::from(capture_source_id.value()),
                ),
                KeyValue::new(STREAM_ID_ATTRIBUTE, i64::from(stream_id.value())),
            ],
        );
    }

    fn record_encoded_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        let target = *self.prj_ref().as_ref();
        let MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        } = target
        else {
            unreachable!("encoded frames require a stream metrics target");
        };
        record_completed_source_frame(target, SourceFrameStage::Encode, frame_id);

        Instruments::global().encode_duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[
                KeyValue::new(
                    CAPTURE_SOURCE_ID_ATTRIBUTE,
                    i64::from(capture_source_id.value()),
                ),
                KeyValue::new(STREAM_ID_ATTRIBUTE, i64::from(stream_id.value())),
            ],
        );
    }
}

impl<Deps> NetworkMetricsRecorder for OpenTelemetryMetricsRecorderImpl<Deps> {
    fn register_network_queue_usage(&self, usage: ResourceUsage) {
        *network_queue_usage()
            .lock()
            .expect("network queue usage mutex should not be poisoned") = Some(usage);
    }

    fn unregister_network_queue_usage(&self) {
        network_queue_usage()
            .lock()
            .expect("network queue usage mutex should not be poisoned")
            .take();
    }

    fn record_packetized_frame(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        frame_id: FrameId,
        duration: std::time::Duration,
    ) {
        let target = MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        };
        record_completed_source_frame(target, SourceFrameStage::Packetize, frame_id);
        Instruments::global()
            .packetize_duration
            .record(duration.as_secs_f64() * 1_000.0, &target_attributes(target));
    }

    fn record_sent_bytes(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    ) {
        let target = MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        };
        let mut counts = sent_byte_counts()
            .lock()
            .expect("sent byte count mutex should not be poisoned");
        let count = counts.entry(target).or_default();
        *count = count.saturating_add(bytes as u64);
    }
}

pub(super) fn take_completed_source_frame_counts()
-> HashMap<MetricsTarget, CompletedSourceFrameCounts> {
    let completed_frames = std::mem::take(
        &mut *completed_source_frames()
            .lock()
            .expect("completed source frame registry mutex should not be poisoned"),
    );

    completed_frames
        .into_iter()
        .map(|(target, frames)| {
            (
                target,
                CompletedSourceFrameCounts {
                    captured: frames.captured.len(),
                    encoded: frames.encoded.len(),
                    packetized: frames.packetized.len(),
                },
            )
        })
        .collect()
}

pub(super) fn snapshot_metrics_resource_usages()
-> HashMap<MetricsTarget, TargetResourceUsageSnapshots> {
    metrics_resource_usages()
        .lock()
        .expect("metrics resource usage registry mutex should not be poisoned")
        .iter()
        .map(|(target, usages)| {
            (
                *target,
                TargetResourceUsageSnapshots {
                    capture_pool: usages.capture_pool.as_ref().map(ResourceUsage::snapshot),
                },
            )
        })
        .collect()
}

pub(super) fn snapshot_network_queue_usage() -> Option<ResourceUsageSnapshot> {
    network_queue_usage()
        .lock()
        .expect("network queue usage mutex should not be poisoned")
        .as_ref()
        .map(ResourceUsage::snapshot)
}

pub(super) fn take_sent_byte_counts() -> HashMap<MetricsTarget, u64> {
    std::mem::take(
        &mut *sent_byte_counts()
            .lock()
            .expect("sent byte count mutex should not be poisoned"),
    )
}

pub(super) fn with_registered_metrics_targets<R>(
    f: impl FnOnce(&HashMap<MetricsTarget, usize>) -> R,
) -> R {
    let targets = registered_metrics_targets()
        .lock()
        .expect("metrics target registry mutex should not be poisoned");

    f(&targets)
}

fn registered_metrics_targets() -> &'static Mutex<HashMap<MetricsTarget, usize>> {
    static TARGETS: OnceLock<Mutex<HashMap<MetricsTarget, usize>>> = OnceLock::new();

    TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_completed_source_frame(
    target: MetricsTarget,
    stage: SourceFrameStage,
    frame_id: FrameId,
) {
    let mut completed_frames = completed_source_frames()
        .lock()
        .expect("completed source frame registry mutex should not be poisoned");
    let frames = completed_frames.entry(target).or_default();

    match stage {
        SourceFrameStage::Capture => frames.captured.insert(frame_id),
        SourceFrameStage::Encode => frames.encoded.insert(frame_id),
        SourceFrameStage::Packetize => frames.packetized.insert(frame_id),
    };
}

fn completed_source_frames() -> &'static Mutex<HashMap<MetricsTarget, CompletedSourceFrameIds>> {
    static COMPLETED_FRAMES: OnceLock<Mutex<HashMap<MetricsTarget, CompletedSourceFrameIds>>> =
        OnceLock::new();

    COMPLETED_FRAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn metrics_resource_usages() -> &'static Mutex<HashMap<MetricsTarget, TargetResourceUsages>> {
    static RESOURCE_USAGES: OnceLock<Mutex<HashMap<MetricsTarget, TargetResourceUsages>>> =
        OnceLock::new();

    RESOURCE_USAGES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn network_queue_usage() -> &'static Mutex<Option<ResourceUsage>> {
    static USAGE: OnceLock<Mutex<Option<ResourceUsage>>> = OnceLock::new();
    USAGE.get_or_init(|| Mutex::new(None))
}

fn sent_byte_counts() -> &'static Mutex<HashMap<MetricsTarget, u64>> {
    static COUNTS: OnceLock<Mutex<HashMap<MetricsTarget, u64>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clear_completed_source_frames(target: MetricsTarget) {
    completed_source_frames()
        .lock()
        .expect("completed source frame registry mutex should not be poisoned")
        .remove(&target);
}

fn clear_metrics_resource_usages(target: MetricsTarget) {
    metrics_resource_usages()
        .lock()
        .expect("metrics resource usage registry mutex should not be poisoned")
        .remove(&target);
}

fn target_attributes(target: MetricsTarget) -> Vec<KeyValue> {
    match target {
        MetricsTarget::CaptureSource(capture_source_id) => vec![KeyValue::new(
            CAPTURE_SOURCE_ID_ATTRIBUTE,
            i64::from(capture_source_id.value()),
        )],
        MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        } => vec![
            KeyValue::new(
                CAPTURE_SOURCE_ID_ATTRIBUTE,
                i64::from(capture_source_id.value()),
            ),
            KeyValue::new(STREAM_ID_ATTRIBUTE, i64::from(stream_id.value())),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

    #[test]
    fn counts_each_source_frame_once_per_stage() {
        let capture_source_id = CaptureSourceId::new(42);
        let target = MetricsTarget::Stream {
            capture_source_id,
            stream_id: StreamId::new(7),
        };
        let first_frame = FrameId::new(capture_source_id, 1);
        let second_frame = FrameId::new(capture_source_id, 2);

        record_completed_source_frame(target, SourceFrameStage::Encode, first_frame);
        record_completed_source_frame(target, SourceFrameStage::Encode, first_frame);
        record_completed_source_frame(target, SourceFrameStage::Encode, second_frame);
        record_completed_source_frame(target, SourceFrameStage::Packetize, first_frame);

        let counts = take_completed_source_frame_counts();
        let counts = counts
            .get(&target)
            .expect("stream completion counts should exist");

        assert_eq!(counts.encoded, 2);
        assert_eq!(counts.packetized, 1);
    }
}
