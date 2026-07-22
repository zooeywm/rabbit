use std::{
    fmt::{self, Display, Formatter},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct VideoCaptureTiming {
    pub(crate) vblank_wait_started: Instant,
    pub(crate) capture_started: Instant,
    pub(crate) capture_completed: Instant,
}

#[derive(Debug)]
pub(crate) struct VideoProbeClock {
    epoch: Instant,
    next_frame_id: u64,
    report_interval: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct VideoFrameProbe {
    frame_id: u64,
    pts_ns: u64,
    timestamps: VideoFrameTimestamps,
    report_interval: Duration,
}

#[derive(Debug, Clone)]
struct VideoFrameTimestamps {
    vblank_wait_started: Instant,
    capture_started: Instant,
    capture_completed: Instant,
    gpu_received: Option<Instant>,
    gpu_submitted: Option<Instant>,
    pipeline_ready: Option<Instant>,
    encoder_submitted: Option<Instant>,
    vpp_entered: Option<Instant>,
    vpp_completed: Option<Instant>,
    encoder_entered: Option<Instant>,
    encoder_completed: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct VideoProbeReporter {
    report_interval: Duration,
    window_started: Option<Instant>,
    frames: u64,
    rtp_packets: u64,
    rtp_bytes: u64,
    totals: VideoProbeStageTotals,
}

#[derive(Debug, Default)]
struct VideoProbeStageTotals {
    vblank_wait: Duration,
    capture: Duration,
    capture_queue: Duration,
    gpu_process: Duration,
    gpu_fence: Duration,
    encoder_queue: Duration,
    vpp_queue: Duration,
    vpp: Duration,
    encode_queue: Duration,
    encode: Duration,
    rtp_packetize: Duration,
    host_latency: Duration,
}

struct VideoFrameTimings {
    vblank_wait: Duration,
    capture: Duration,
    capture_queue: Duration,
    gpu_process: Duration,
    gpu_fence: Duration,
    encoder_queue: Duration,
    vpp_queue: Duration,
    vpp: Duration,
    encode_queue: Duration,
    encode: Duration,
    rtp_packetize: Duration,
    host_latency: Duration,
}

struct TwoDecimal(f64);

impl Display for TwoDecimal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:.2}", self.0)
    }
}

impl VideoProbeClock {
    pub(crate) fn new(report_interval: Duration) -> Self {
        Self {
            epoch: Instant::now(),
            next_frame_id: 0,
            report_interval,
        }
    }

    pub(crate) fn frame(&mut self, timing: VideoCaptureTiming) -> VideoFrameProbe {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.saturating_add(1);

        VideoFrameProbe {
            frame_id,
            pts_ns: duration_ns(timing.capture_started.duration_since(self.epoch)),
            report_interval: self.report_interval,
            timestamps: VideoFrameTimestamps {
                vblank_wait_started: timing.vblank_wait_started,
                capture_started: timing.capture_started,
                capture_completed: timing.capture_completed,
                gpu_received: None,
                gpu_submitted: None,
                pipeline_ready: None,
                encoder_submitted: None,
                vpp_entered: None,
                vpp_completed: None,
                encoder_entered: None,
                encoder_completed: None,
            },
        }
    }
}

impl VideoFrameProbe {
    pub(crate) fn frame_id(&self) -> u64 {
        self.frame_id
    }

    pub(crate) fn pts_ns(&self) -> u64 {
        self.pts_ns
    }

    pub(crate) fn report_interval(&self) -> Duration {
        self.report_interval
    }

    pub(crate) fn mark_gpu_received(&mut self) {
        self.timestamps.gpu_received = Some(Instant::now());
    }

    pub(crate) fn mark_gpu_submitted(&mut self) {
        self.timestamps.gpu_submitted = Some(Instant::now());
    }

    pub(crate) fn mark_pipeline_ready(&mut self) {
        self.timestamps.pipeline_ready = Some(Instant::now());
    }

    pub(crate) fn mark_encoder_submitted(&mut self) {
        self.timestamps.encoder_submitted = Some(Instant::now());
    }

    pub(crate) fn mark_vpp_entered(&mut self, at: Instant) {
        self.timestamps.vpp_entered = Some(at);
    }

    pub(crate) fn mark_vpp_completed(&mut self, at: Instant) {
        self.timestamps.vpp_completed = Some(at);
    }

    pub(crate) fn mark_encoder_entered(&mut self, at: Instant) {
        self.timestamps.encoder_entered = Some(at);
    }

    pub(crate) fn mark_encoder_completed(&mut self, at: Option<Instant>) {
        self.timestamps.encoder_completed = at;
    }

    fn finish(&self, rtp_completed: Instant) -> Result<VideoFrameTimings, &'static str> {
        let timestamps = &self.timestamps;
        let gpu_received = required(timestamps.gpu_received, "gpu_received")?;
        let gpu_submitted = required(timestamps.gpu_submitted, "gpu_submitted")?;
        let pipeline_ready = required(timestamps.pipeline_ready, "pipeline_ready")?;
        let encoder_submitted = required(timestamps.encoder_submitted, "encoder_submitted")?;
        let (vpp_queue, vpp, vpp_completed) = match timestamps.vpp_entered {
            Some(vpp_entered) => {
                let vpp_completed = required(timestamps.vpp_completed, "vpp_completed")?;
                (
                    elapsed(encoder_submitted, vpp_entered),
                    elapsed(vpp_entered, vpp_completed),
                    vpp_completed,
                )
            }
            None => (Duration::ZERO, Duration::ZERO, encoder_submitted),
        };
        let encoder_entered = required(timestamps.encoder_entered, "encoder_entered")?;
        let encoder_completed = required(timestamps.encoder_completed, "encoder_completed")?;

        Ok(VideoFrameTimings {
            vblank_wait: elapsed(timestamps.vblank_wait_started, timestamps.capture_started),
            capture: elapsed(timestamps.capture_started, timestamps.capture_completed),
            capture_queue: elapsed(timestamps.capture_completed, gpu_received),
            gpu_process: elapsed(gpu_received, gpu_submitted),
            gpu_fence: elapsed(gpu_submitted, pipeline_ready),
            encoder_queue: elapsed(pipeline_ready, encoder_submitted),
            vpp_queue,
            vpp,
            encode_queue: elapsed(vpp_completed, encoder_entered),
            encode: elapsed(encoder_entered, encoder_completed),
            rtp_packetize: elapsed(encoder_completed, rtp_completed),
            host_latency: elapsed(timestamps.capture_started, rtp_completed),
        })
    }
}

impl VideoProbeReporter {
    pub(crate) fn new(report_interval: Duration) -> Self {
        Self {
            report_interval,
            window_started: None,
            frames: 0,
            rtp_packets: 0,
            rtp_bytes: 0,
            totals: VideoProbeStageTotals::default(),
        }
    }

    pub(crate) fn record_frame(
        &mut self,
        probe: VideoFrameProbe,
        rtp_packets: u64,
        rtp_bytes: u64,
    ) {
        let now = Instant::now();
        let timings = match probe.finish(now) {
            Ok(timings) => timings,
            Err(stage) => {
                tracing::warn!(
                    target: "rabbit::video_probe",
                    frame_id = probe.frame_id(),
                    missing_stage = stage,
                    "Video frame probe is incomplete"
                );
                return;
            }
        };

        tracing::trace!(
            target: "rabbit::video_probe",
            frame_id = probe.frame_id(),
            pts_ns = probe.pts_ns(),
            vblank_wait_ms = %TwoDecimal(duration_ms(timings.vblank_wait)),
            capture_ms = %TwoDecimal(duration_ms(timings.capture)),
            capture_queue_ms = %TwoDecimal(duration_ms(timings.capture_queue)),
            gpu_process_ms = %TwoDecimal(duration_ms(timings.gpu_process)),
            gpu_fence_ms = %TwoDecimal(duration_ms(timings.gpu_fence)),
            encoder_queue_ms = %TwoDecimal(duration_ms(timings.encoder_queue)),
            vpp_queue_ms = %TwoDecimal(duration_ms(timings.vpp_queue)),
            vpp_ms = %TwoDecimal(duration_ms(timings.vpp)),
            encode_queue_ms = %TwoDecimal(duration_ms(timings.encode_queue)),
            encode_ms = %TwoDecimal(duration_ms(timings.encode)),
            rtp_packetize_ms = %TwoDecimal(duration_ms(timings.rtp_packetize)),
            host_latency_ms = %TwoDecimal(duration_ms(timings.host_latency)),
            rtp_packets,
            rtp_bytes,
            "Video frame encoded"
        );

        self.window_started.get_or_insert(now);
        self.frames += 1;
        self.rtp_packets += rtp_packets;
        self.rtp_bytes += rtp_bytes;
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
    }

    fn report_window(&mut self, partial: bool) {
        let Some(started) = self.window_started else {
            return;
        };
        if self.frames == 0 {
            return;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(started);
        let frames = self.frames;

        tracing::info!(
            target: "rabbit::video_probe",
            partial,
            window_ms = %TwoDecimal(duration_ms(elapsed)),
            frames,
            fps = %TwoDecimal(rate(frames, elapsed)),
            avg_host_latency_ms = %TwoDecimal(average_ms(self.totals.host_latency, frames)),
            avg_vblank_wait_ms = %TwoDecimal(average_ms(self.totals.vblank_wait, frames)),
            avg_capture_ms = %TwoDecimal(average_ms(self.totals.capture, frames)),
            avg_capture_queue_ms = %TwoDecimal(average_ms(self.totals.capture_queue, frames)),
            avg_gpu_process_ms = %TwoDecimal(average_ms(self.totals.gpu_process, frames)),
            avg_gpu_fence_ms = %TwoDecimal(average_ms(self.totals.gpu_fence, frames)),
            avg_encoder_queue_ms = %TwoDecimal(average_ms(self.totals.encoder_queue, frames)),
            avg_vpp_queue_ms = %TwoDecimal(average_ms(self.totals.vpp_queue, frames)),
            avg_vpp_ms = %TwoDecimal(average_ms(self.totals.vpp, frames)),
            avg_encode_queue_ms = %TwoDecimal(average_ms(self.totals.encode_queue, frames)),
            avg_encode_ms = %TwoDecimal(average_ms(self.totals.encode, frames)),
            avg_rtp_packetize_ms = %TwoDecimal(average_ms(self.totals.rtp_packetize, frames)),
            rtp_packets = self.rtp_packets,
            rtp_bytes = self.rtp_bytes,
            "Video throughput window"
        );

        self.window_started = Some(now);
        self.frames = 0;
        self.rtp_packets = 0;
        self.rtp_bytes = 0;
        self.totals = VideoProbeStageTotals::default();
    }
}

impl VideoProbeStageTotals {
    fn add(&mut self, timings: &VideoFrameTimings) {
        self.vblank_wait += timings.vblank_wait;
        self.capture += timings.capture;
        self.capture_queue += timings.capture_queue;
        self.gpu_process += timings.gpu_process;
        self.gpu_fence += timings.gpu_fence;
        self.encoder_queue += timings.encoder_queue;
        self.vpp_queue += timings.vpp_queue;
        self.vpp += timings.vpp;
        self.encode_queue += timings.encode_queue;
        self.encode += timings.encode;
        self.rtp_packetize += timings.rtp_packetize;
        self.host_latency += timings.host_latency;
    }
}

fn required(timestamp: Option<Instant>, stage: &'static str) -> Result<Instant, &'static str> {
    timestamp.ok_or(stage)
}

fn elapsed(start: Instant, end: Instant) -> Duration {
    end.checked_duration_since(start).unwrap_or(Duration::ZERO)
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
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

// Focused test: cargo test infra::platform::video_probe::tests:: --lib
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::infra::platform::video_probe::{TwoDecimal, VideoProbeReporter};

    #[test]
    fn uses_the_configured_host_video_interval() {
        let reporter = VideoProbeReporter::new(Duration::from_millis(750));

        assert_eq!(reporter.report_interval, Duration::from_millis(750));
    }

    #[test]
    fn formats_host_video_floats_with_two_decimal_places() {
        assert_eq!(TwoDecimal(115.12956516018339).to_string(), "115.13");
    }
}
