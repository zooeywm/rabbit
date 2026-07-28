use std::{
    fmt::{self, Display, Formatter},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub(crate) struct HostVideoFrameProbe {
    frame_id: u64,
    report_interval: Duration,
    timestamps: HostVideoFrameTimestamps,
}

#[derive(Debug, Clone)]
struct HostVideoFrameTimestamps {
    capture_received: Instant,
    pipeline_ready: Option<Instant>,
    encoder_received: Option<Instant>,
    vpp_started: Option<Instant>,
    vpp_completed: Option<Instant>,
    encoder_submitted: Option<Instant>,
    encoder_completed: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct HostVideoProbeReporter {
    report_interval: Duration,
    window_started: Option<Instant>,
    frames: u64,
    rtp_packets: u64,
    rtp_bytes: u64,
    totals: HostVideoProbeStageTotals,
}

#[derive(Debug, Default)]
struct HostVideoProbeStageTotals {
    capture_queue: Duration,
    encoder_queue: Duration,
    encoder_wait: Duration,
    vpp: Duration,
    encode: Duration,
    rtp_send: Duration,
    host_latency: Duration,
}

struct HostVideoFrameTimings {
    capture_queue: Duration,
    encoder_queue: Duration,
    encoder_wait: Duration,
    vpp: Duration,
    encode: Duration,
    rtp_send: Duration,
    host_latency: Duration,
}

struct TwoDecimal(f64);

impl Display for TwoDecimal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:.2}", self.0)
    }
}

impl HostVideoFrameProbe {
    pub(crate) fn new(frame_id: u64, report_interval: Duration) -> Self {
        Self {
            frame_id,
            report_interval,
            timestamps: HostVideoFrameTimestamps {
                capture_received: Instant::now(),
                pipeline_ready: None,
                encoder_received: None,
                vpp_started: None,
                vpp_completed: None,
                encoder_submitted: None,
                encoder_completed: None,
            },
        }
    }

    pub(crate) fn report_interval(&self) -> Duration {
        self.report_interval
    }

    pub(crate) fn mark_pipeline_ready(&mut self) {
        self.timestamps.pipeline_ready = Some(Instant::now());
    }

    pub(crate) fn mark_encoder_received(&mut self) {
        self.timestamps.encoder_received = Some(Instant::now());
    }

    pub(crate) fn mark_vpp_started(&mut self) {
        self.timestamps.vpp_started = Some(Instant::now());
    }

    pub(crate) fn mark_vpp_completed(&mut self) {
        self.timestamps.vpp_completed = Some(Instant::now());
    }

    pub(crate) fn mark_encoder_submitted(&mut self) {
        self.timestamps.encoder_submitted = Some(Instant::now());
    }

    pub(crate) fn mark_encoder_completed(&mut self) {
        self.timestamps.encoder_completed = Some(Instant::now());
    }

    fn finish(&self, rtp_completed: Instant) -> Result<HostVideoFrameTimings, &'static str> {
        let timestamps = &self.timestamps;
        let pipeline_ready = required(timestamps.pipeline_ready, "pipeline_ready")?;
        let encoder_received = required(timestamps.encoder_received, "encoder_received")?;
        let vpp_started = required(timestamps.vpp_started, "vpp_started")?;
        let vpp_completed = required(timestamps.vpp_completed, "vpp_completed")?;
        let encoder_submitted = required(timestamps.encoder_submitted, "encoder_submitted")?;
        let encoder_completed = required(timestamps.encoder_completed, "encoder_completed")?;

        Ok(HostVideoFrameTimings {
            capture_queue: elapsed(timestamps.capture_received, pipeline_ready),
            encoder_queue: elapsed(pipeline_ready, encoder_received),
            encoder_wait: elapsed(encoder_received, vpp_started),
            vpp: elapsed(vpp_started, vpp_completed),
            encode: elapsed(encoder_submitted, encoder_completed),
            rtp_send: elapsed(encoder_completed, rtp_completed),
            host_latency: elapsed(timestamps.capture_received, rtp_completed),
        })
    }
}

impl HostVideoProbeReporter {
    pub(crate) fn new(report_interval: Duration) -> Self {
        Self {
            report_interval,
            window_started: None,
            frames: 0,
            rtp_packets: 0,
            rtp_bytes: 0,
            totals: HostVideoProbeStageTotals::default(),
        }
    }

    pub(crate) fn record_frame(
        &mut self,
        probe: HostVideoFrameProbe,
        rtp_packets: u64,
        rtp_bytes: u64,
    ) {
        let now = Instant::now();
        let timings = match probe.finish(now) {
            Ok(timings) => timings,
            Err(stage) => {
                tracing::warn!(
                    target: "rabbit::video_probe",
                    platform = "windows",
                    frame_id = probe.frame_id,
                    missing_stage = stage,
                    "Host video frame probe is incomplete"
                );
                return;
            }
        };

        tracing::trace!(
            target: "rabbit::video_probe",
            platform = "windows",
            frame_id = probe.frame_id,
            capture_queue_ms = %TwoDecimal(duration_ms(timings.capture_queue)),
            encoder_queue_ms = %TwoDecimal(duration_ms(timings.encoder_queue)),
            encoder_wait_ms = %TwoDecimal(duration_ms(timings.encoder_wait)),
            vpp_ms = %TwoDecimal(duration_ms(timings.vpp)),
            encode_ms = %TwoDecimal(duration_ms(timings.encode)),
            rtp_send_ms = %TwoDecimal(duration_ms(timings.rtp_send)),
            host_latency_ms = %TwoDecimal(duration_ms(timings.host_latency)),
            rtp_packets,
            rtp_bytes,
            "Host video frame encoded"
        );

        self.window_started.get_or_insert(now);
        self.frames = self.frames.saturating_add(1);
        self.rtp_packets = self.rtp_packets.saturating_add(rtp_packets);
        self.rtp_bytes = self.rtp_bytes.saturating_add(rtp_bytes);
        self.totals.add(&timings);

        if self
            .window_started
            .is_some_and(|started| now.duration_since(started) >= self.report_interval)
        {
            self.report_window(false);
        }
    }

    pub(crate) fn finish(&mut self) {
        self.report_window(true);
        self.window_started = None;
    }

    fn report_window(&mut self, partial: bool) {
        let Some(started) = self.window_started else {
            return;
        };
        if self.frames == 0 {
            return;
        }
        let now = Instant::now();
        let window_elapsed = now.duration_since(started);
        let frames = self.frames;
        tracing::info!(
            target: "rabbit::video_probe",
            platform = "windows",
            partial,
            window_ms = %TwoDecimal(duration_ms(window_elapsed)),
            frames,
            fps = %TwoDecimal(rate(frames, window_elapsed)),
            avg_host_latency_ms = %TwoDecimal(average_ms(self.totals.host_latency, frames)),
            avg_capture_queue_ms = %TwoDecimal(average_ms(self.totals.capture_queue, frames)),
            avg_encoder_queue_ms = %TwoDecimal(average_ms(self.totals.encoder_queue, frames)),
            avg_encoder_wait_ms = %TwoDecimal(average_ms(self.totals.encoder_wait, frames)),
            avg_vpp_ms = %TwoDecimal(average_ms(self.totals.vpp, frames)),
            avg_encode_ms = %TwoDecimal(average_ms(self.totals.encode, frames)),
            avg_rtp_send_ms = %TwoDecimal(average_ms(self.totals.rtp_send, frames)),
            rtp_packets = self.rtp_packets,
            rtp_bytes = self.rtp_bytes,
            "Host video throughput window"
        );

        self.window_started = Some(now);
        self.frames = 0;
        self.rtp_packets = 0;
        self.rtp_bytes = 0;
        self.totals = HostVideoProbeStageTotals::default();
    }
}

impl HostVideoProbeStageTotals {
    fn add(&mut self, timings: &HostVideoFrameTimings) {
        self.capture_queue += timings.capture_queue;
        self.encoder_queue += timings.encoder_queue;
        self.encoder_wait += timings.encoder_wait;
        self.vpp += timings.vpp;
        self.encode += timings.encode;
        self.rtp_send += timings.rtp_send;
        self.host_latency += timings.host_latency;
    }
}

fn required(timestamp: Option<Instant>, stage: &'static str) -> Result<Instant, &'static str> {
    timestamp.ok_or(stage)
}

fn elapsed(start: Instant, end: Instant) -> Duration {
    end.checked_duration_since(start).unwrap_or(Duration::ZERO)
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn average_ms(total: Duration, count: u64) -> f64 {
    duration_ms(total) / count as f64
}

fn rate(count: u64, duration: Duration) -> f64 {
    if duration.is_zero() {
        return 0.0;
    }
    count as f64 / duration.as_secs_f64()
}
