use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use opentelemetry::{KeyValue, Value, global};
use opentelemetry_sdk::{
    error::{OTelSdkError, OTelSdkResult},
    metrics::{
        Aggregation, Instrument, PeriodicReader, SdkMeterProvider, Stream, Temporality,
        data::{
            AggregatedMetrics, ExponentialHistogramDataPoint, Metric, MetricData, ResourceMetrics,
        },
        exporter::PushMetricExporter,
    },
};

use crate::{
    app::container::root::outbound_port::{MetricsTarget, ResourceUsageSnapshot},
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

use super::metrics_recorder::{
    CAPTURE_DURATION_METRIC, CAPTURE_SOURCE_ID_ATTRIBUTE, CONVERT_DURATION_METRIC,
    CompletedSourceFrameCounts, ENCODE_DURATION_METRIC, PACKETIZE_DURATION_METRIC,
    STREAM_ID_ATTRIBUTE, TargetResourceUsageSnapshots, snapshot_metrics_resource_usages,
    take_completed_source_frame_counts, with_registered_metrics_targets,
};

struct TracingMetricExporter {
    is_shutdown: AtomicBool,
    last_exported_at: Mutex<Instant>,
}

pub(crate) struct MetricsRuntime {
    meter_provider: SdkMeterProvider,
}

pub(crate) fn init_metrics() -> MetricsRuntime {
    let reader = PeriodicReader::builder(TracingMetricExporter::default()).build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_view(duration_histogram_view)
        .build();

    global::set_meter_provider(meter_provider.clone());
    tracing::info!(
        target: "rabbit::metrics",
        "runtime metrics initialized: ms=(avg p50 p95 p99)"
    );

    MetricsRuntime { meter_provider }
}

impl Default for TracingMetricExporter {
    fn default() -> Self {
        Self {
            is_shutdown: AtomicBool::new(false),
            last_exported_at: Mutex::new(Instant::now()),
        }
    }
}

impl PushMetricExporter for TracingMetricExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(OTelSdkError::AlreadyShutdown);
        }

        let export_period = self.take_export_period();
        let completed_source_frames = take_completed_source_frame_counts();
        let resource_usages = snapshot_metrics_resource_usages();
        with_registered_metrics_targets(|targets| {
            export_registered_targets(
                metrics,
                targets,
                &completed_source_frames,
                &resource_usages,
                export_period,
            );
        });

        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        if self.is_shutdown.load(Ordering::Acquire) {
            Err(OTelSdkError::AlreadyShutdown)
        } else {
            Ok(())
        }
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.is_shutdown
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| OTelSdkError::AlreadyShutdown)
    }

    fn temporality(&self) -> Temporality {
        Temporality::LowMemory
    }
}

impl TracingMetricExporter {
    fn take_export_period(&self) -> Duration {
        let now = Instant::now();
        let mut last_exported_at = self
            .last_exported_at
            .lock()
            .expect("metrics export time mutex should not be poisoned");
        let export_period = now.duration_since(*last_exported_at);

        *last_exported_at = now;
        export_period
    }
}

impl Drop for MetricsRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.meter_provider.shutdown() {
            tracing::error!(%error, "failed to shut down metrics runtime");
        }
    }
}

fn duration_histogram_view(instrument: &Instrument) -> Option<Stream> {
    match instrument.name() {
        CAPTURE_DURATION_METRIC
        | CONVERT_DURATION_METRIC
        | ENCODE_DURATION_METRIC
        | PACKETIZE_DURATION_METRIC => Some(
            Stream::builder()
                .with_aggregation(Aggregation::Base2ExponentialHistogram {
                    max_size: 160,
                    max_scale: 8,
                    record_min_max: true,
                })
                .build()
                .expect("duration histogram view should be valid"),
        ),
        _ => None,
    }
}

#[derive(Default)]
struct SourceMetrics {
    completed_capture_frames: usize,
    capture_pool: ResourceUsageSnapshot,
    capture_duration: DurationMetrics,
    streams: HashMap<StreamId, StreamMetrics>,
}

#[derive(Default)]
struct StreamMetrics {
    completed_encoded_frames: usize,
    completed_packetized_frames: usize,
    packetizer_queue: ResourceUsageSnapshot,
    convert_duration: DurationMetrics,
    encode_duration: DurationMetrics,
    packetize_duration: DurationMetrics,
}

#[derive(Clone, Copy, Default)]
struct DurationMetrics {
    average_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

fn export_registered_targets(
    resource_metrics: &ResourceMetrics,
    targets: &HashMap<MetricsTarget, usize>,
    completed_source_frames: &HashMap<MetricsTarget, CompletedSourceFrameCounts>,
    resource_usages: &HashMap<MetricsTarget, TargetResourceUsageSnapshots>,
    export_period: Duration,
) {
    let mut source_metrics = HashMap::<CaptureSourceId, SourceMetrics>::new();

    for target in targets.keys() {
        match target {
            MetricsTarget::CaptureSource(capture_source_id) => {
                source_metrics.entry(*capture_source_id).or_default();
            }
            MetricsTarget::Stream {
                capture_source_id,
                stream_id,
            } => {
                source_metrics
                    .entry(*capture_source_id)
                    .or_default()
                    .streams
                    .entry(*stream_id)
                    .or_default();
            }
        }
    }

    if source_metrics.is_empty() {
        return;
    }

    for metric in resource_metrics
        .scope_metrics()
        .flat_map(|scope_metrics| scope_metrics.metrics())
    {
        aggregate_duration_metric(metric, targets, &mut source_metrics);
    }

    for (target, completed_frames) in completed_source_frames {
        if !targets.contains_key(target) {
            continue;
        }

        match target {
            MetricsTarget::CaptureSource(capture_source_id) => {
                source_metrics
                    .get_mut(capture_source_id)
                    .expect("registered capture source metrics should exist")
                    .completed_capture_frames = completed_frames.captured;
            }
            MetricsTarget::Stream {
                capture_source_id,
                stream_id,
            } => {
                let stream = source_metrics
                    .get_mut(capture_source_id)
                    .and_then(|source| source.streams.get_mut(stream_id))
                    .expect("registered stream metrics should exist");

                stream.completed_encoded_frames = completed_frames.encoded;
                stream.completed_packetized_frames = completed_frames.packetized;
            }
        }
    }

    for (target, usages) in resource_usages {
        if !targets.contains_key(target) {
            continue;
        }

        match target {
            MetricsTarget::CaptureSource(capture_source_id) => {
                if let Some(capture_pool) = usages.capture_pool {
                    source_metrics
                        .get_mut(capture_source_id)
                        .expect("registered capture source metrics should exist")
                        .capture_pool = capture_pool;
                }
            }
            MetricsTarget::Stream {
                capture_source_id,
                stream_id,
            } => {
                if let Some(packetizer_queue) = usages.packetizer_queue {
                    source_metrics
                        .get_mut(capture_source_id)
                        .and_then(|source| source.streams.get_mut(stream_id))
                        .expect("registered stream metrics should exist")
                        .packetizer_queue = packetizer_queue;
                }
            }
        }
    }

    let mut capture_source_ids = source_metrics.keys().copied().collect::<Vec<_>>();
    capture_source_ids.sort_unstable_by_key(|capture_source_id| capture_source_id.value());

    let sources = capture_source_ids
        .into_iter()
        .map(|capture_source_id| {
            let values = &source_metrics[&capture_source_id];
            let capture_fps =
                frames_per_second(values.completed_capture_frames, export_period);
            let capture_ms = format_duration(values.capture_duration);
            let mut streams = values.streams.iter().collect::<Vec<_>>();

            streams.sort_unstable_by_key(|(stream_id, _)| stream_id.value());

            let streams = streams
                .into_iter()
                .map(|(stream_id, values)| {
                    format!(
                        "{{id={} enc_fps={:.2} cvt_ms={} enc_ms={} pkt_fps={:.2} pkt_queue={}/{} pkt_ms={}}}",
                        stream_id.value(),
                        frames_per_second(values.completed_encoded_frames, export_period),
                        format_duration(values.convert_duration),
                        format_duration(values.encode_duration),
                        frames_per_second(values.completed_packetized_frames, export_period),
                        values.packetizer_queue.used,
                        values.packetizer_queue.total,
                        format_duration(values.packetize_duration),
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let streams = format!("[{streams}]");

            format!(
                "{{id={} cap_fps={capture_fps:.2} cap_pool={}/{} cap_ms={} streams={streams}}}",
                capture_source_id.value(),
                values.capture_pool.used,
                values.capture_pool.total,
                capture_ms,
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    tracing::info!(
        target: "rabbit::metrics",
        "runtime metrics: sources=[{sources}]"
    );
}

fn aggregate_duration_metric(
    metric: &Metric,
    targets: &HashMap<MetricsTarget, usize>,
    source_metrics: &mut HashMap<CaptureSourceId, SourceMetrics>,
) {
    let AggregatedMetrics::F64(MetricData::ExponentialHistogram(histogram)) = metric.data() else {
        return;
    };

    for point in histogram.data_points() {
        let Some(target) = metric_target(metric.name(), point.attributes()) else {
            continue;
        };
        if !targets.contains_key(&target) {
            continue;
        }

        let duration = summarize_duration(point);

        match (metric.name(), target) {
            (CAPTURE_DURATION_METRIC, MetricsTarget::CaptureSource(capture_source_id)) => {
                source_metrics
                    .get_mut(&capture_source_id)
                    .expect("registered capture source metrics should exist")
                    .capture_duration = duration;
            }
            (
                CONVERT_DURATION_METRIC | ENCODE_DURATION_METRIC | PACKETIZE_DURATION_METRIC,
                MetricsTarget::Stream {
                    capture_source_id,
                    stream_id,
                },
            ) => {
                let values = source_metrics
                    .get_mut(&capture_source_id)
                    .and_then(|source| source.streams.get_mut(&stream_id))
                    .expect("registered stream metrics should exist");

                match metric.name() {
                    CONVERT_DURATION_METRIC => values.convert_duration = duration,
                    ENCODE_DURATION_METRIC => values.encode_duration = duration,
                    PACKETIZE_DURATION_METRIC => values.packetize_duration = duration,
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }
}

fn summarize_duration(point: &ExponentialHistogramDataPoint<f64>) -> DurationMetrics {
    if point.count() == 0 {
        return DurationMetrics::default();
    }

    DurationMetrics {
        average_ms: point.sum() / point.count() as f64,
        p50_ms: exponential_histogram_quantile(point, 0.50),
        p95_ms: exponential_histogram_quantile(point, 0.95),
        p99_ms: exponential_histogram_quantile(point, 0.99),
    }
}

fn exponential_histogram_quantile(
    point: &ExponentialHistogramDataPoint<f64>,
    quantile: f64,
) -> f64 {
    let rank = (point.count() as f64 * quantile).ceil() as u64;

    if rank <= point.zero_count() {
        return 0.0;
    }

    let mut cumulative_count = point.zero_count();
    let bucket = point.positive_bucket();
    let base_exponent = 2_f64.powi(-i32::from(point.scale()));

    for (bucket_position, bucket_count) in bucket.counts().enumerate() {
        cumulative_count += bucket_count;

        if cumulative_count >= rank {
            let bucket_index = bucket.offset()
                + i32::try_from(bucket_position)
                    .expect("duration histogram bucket position should fit in i32");

            return 2_f64.powf(base_exponent * f64::from(bucket_index + 1));
        }
    }

    unreachable!("duration histogram count should be represented by non-negative buckets");
}

fn frames_per_second(frame_count: usize, export_period: Duration) -> f64 {
    let seconds = export_period.as_secs_f64();

    if seconds == 0.0 {
        0.0
    } else {
        frame_count as f64 / seconds
    }
}

fn format_duration(duration: DurationMetrics) -> String {
    format!(
        "{{{:.3} {:.3} {:.3} {:.3}}}",
        duration.average_ms, duration.p50_ms, duration.p95_ms, duration.p99_ms,
    )
}

fn metric_target<'a>(
    metric_name: &str,
    attributes: impl Iterator<Item = &'a KeyValue>,
) -> Option<MetricsTarget> {
    let mut capture_source_id = None;
    let mut stream_id = None;

    for attribute in attributes {
        let Value::I64(value) = &attribute.value else {
            continue;
        };
        let Ok(value) = u16::try_from(*value) else {
            continue;
        };

        match attribute.key.as_str() {
            CAPTURE_SOURCE_ID_ATTRIBUTE => capture_source_id = Some(CaptureSourceId::new(value)),
            STREAM_ID_ATTRIBUTE => stream_id = Some(StreamId::new(value)),
            _ => {}
        }
    }

    match metric_name {
        CAPTURE_DURATION_METRIC => Some(MetricsTarget::CaptureSource(capture_source_id?)),
        CONVERT_DURATION_METRIC | ENCODE_DURATION_METRIC | PACKETIZE_DURATION_METRIC => {
            Some(MetricsTarget::Stream {
                capture_source_id: capture_source_id?,
                stream_id: stream_id?,
            })
        }
        _ => None,
    }
}
