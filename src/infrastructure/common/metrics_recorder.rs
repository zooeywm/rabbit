use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

use opentelemetry::{
    KeyValue, global,
    metrics::{Histogram, UpDownCounter},
};

use crate::app::container::root::outbound_port::{MetricsRecorder, MetricsTarget};

pub(super) const CAPTURE_SOURCE_ID_ATTRIBUTE: &str = "capture_source_id";
pub(super) const STREAM_ID_ATTRIBUTE: &str = "stream_id";
pub(super) const CAPTURE_DURATION_METRIC: &str = "rabbit.capture.duration";
pub(super) const CONVERT_DURATION_METRIC: &str = "rabbit.convert.duration";
pub(super) const ENCODE_DURATION_METRIC: &str = "rabbit.encode.duration";

#[derive(kudi::DepInj)]
#[target(OpenTelemetryMetricsRecorderImpl)]
pub(crate) struct OpenTelemetryMetricsRecorder;

struct Instruments {
    active_targets: UpDownCounter<i64>,
    capture_duration: Histogram<f64>,
    convert_duration: Histogram<f64>,
    encode_duration: Histogram<f64>,
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
                    .with_description("Time spent converting one frame")
                    .with_unit("ms")
                    .build(),
                encode_duration: meter
                    .f64_histogram(ENCODE_DURATION_METRIC)
                    .with_description("Time spent encoding one frame")
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
        let inserted = registered_metrics_targets()
            .lock()
            .expect("metrics target registry mutex should not be poisoned")
            .insert(*self.prj_ref().as_ref());

        assert!(inserted, "metrics target should only be registered once");

        Instruments::global()
            .active_targets
            .add(1, &target_attributes(*self.prj_ref().as_ref()));
    }

    fn unregister_metrics_target(&self) {
        let removed = registered_metrics_targets()
            .lock()
            .expect("metrics target registry mutex should not be poisoned")
            .remove(self.prj_ref().as_ref());

        assert!(
            removed,
            "metrics target should be registered before removal"
        );

        Instruments::global()
            .active_targets
            .add(-1, &target_attributes(*self.prj_ref().as_ref()));
    }

    fn record_captured_frame(&self, duration: std::time::Duration) {
        let MetricsTarget::CaptureSource(capture_source_id) = self.prj_ref().as_ref() else {
            unreachable!("captured frames require a capture source metrics target");
        };

        Instruments::global().capture_duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[KeyValue::new(
                CAPTURE_SOURCE_ID_ATTRIBUTE,
                i64::from(capture_source_id.value()),
            )],
        );
    }

    fn record_converted_frame(&self, duration: std::time::Duration) {
        let MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        } = self.prj_ref().as_ref()
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

    fn record_encoded_frame(&self, duration: std::time::Duration) {
        let MetricsTarget::Stream {
            capture_source_id,
            stream_id,
        } = self.prj_ref().as_ref()
        else {
            unreachable!("encoded frames require a stream metrics target");
        };

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

pub(super) fn with_registered_metrics_targets<R>(
    f: impl FnOnce(&HashSet<MetricsTarget>) -> R,
) -> R {
    let targets = registered_metrics_targets()
        .lock()
        .expect("metrics target registry mutex should not be poisoned");

    f(&targets)
}

fn registered_metrics_targets() -> &'static Mutex<HashSet<MetricsTarget>> {
    static TARGETS: OnceLock<Mutex<HashSet<MetricsTarget>>> = OnceLock::new();

    TARGETS.get_or_init(|| Mutex::new(HashSet::new()))
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
