use std::{
    fmt::{self, Display, Formatter},
    time::{Duration, Instant},
};

use crate::kernel::screen_manager::ScreenId;

#[derive(Debug, Default)]
pub(crate) struct ClientVideoProbeClock {
    next_frame_id: u64,
}

#[derive(Debug)]
pub(crate) struct ClientVideoFrameProbe {
    frame_id: u64,
    rtp_packets: u64,
    rtp_bytes: u64,
    timestamps: ClientVideoFrameTimestamps,
}

#[derive(Debug)]
struct ClientVideoFrameTimestamps {
    decoder_submitted: Instant,
    decoder_entered: Option<Instant>,
    decoder_completed: Option<Instant>,
    gui_received: Option<Instant>,
    render_started: Option<Instant>,
    render_completed: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct ClientVideoProbeReporter {
    report_interval: Duration,
    window_started: Option<Instant>,
    frames: u64,
    rtp_packets: u64,
    rtp_bytes: u64,
    totals: ClientVideoProbeStageTotals,
}

#[derive(Debug, Default)]
struct ClientVideoProbeStageTotals {
    decoder_queue: Duration,
    decode: Duration,
    presentation_queue: Duration,
    render_queue: Duration,
    render: Duration,
    client_latency: Duration,
}

struct ClientVideoFrameTimings {
    decoder_queue: Duration,
    decode: Duration,
    presentation_queue: Duration,
    render_queue: Duration,
    render: Duration,
    client_latency: Duration,
}

struct TwoDecimal(f64);

impl Display for TwoDecimal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:.2}", self.0)
    }
}

impl ClientVideoProbeClock {
    pub(crate) fn frame(&mut self, rtp_packets: usize, rtp_bytes: usize) -> ClientVideoFrameProbe {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        ClientVideoFrameProbe {
            frame_id,
            rtp_packets: u64::try_from(rtp_packets).unwrap_or(u64::MAX),
            rtp_bytes: u64::try_from(rtp_bytes).unwrap_or(u64::MAX),
            timestamps: ClientVideoFrameTimestamps {
                decoder_submitted: Instant::now(),
                decoder_entered: None,
                decoder_completed: None,
                gui_received: None,
                render_started: None,
                render_completed: None,
            },
        }
    }
}

impl ClientVideoFrameProbe {
    pub(crate) fn mark_decoder_entered(&mut self) {
        self.timestamps.decoder_entered = Some(Instant::now());
    }

    pub(crate) fn mark_decoder_completed(&mut self) {
        self.timestamps.decoder_completed = Some(Instant::now());
    }

    pub(crate) fn mark_gui_received(&mut self) {
        self.timestamps.gui_received = Some(Instant::now());
    }

    pub(crate) fn mark_render_started(&mut self) {
        self.timestamps.render_started = Some(Instant::now());
    }

    pub(crate) fn mark_render_completed(&mut self) {
        self.timestamps.render_completed = Some(Instant::now());
    }

    fn finish(&self) -> Result<ClientVideoFrameTimings, &'static str> {
        let timestamps = &self.timestamps;
        let decoder_entered = required(timestamps.decoder_entered, "decoder_entered")?;
        let decoder_completed = required(timestamps.decoder_completed, "decoder_completed")?;
        let gui_received = required(timestamps.gui_received, "gui_received")?;
        let render_started = required(timestamps.render_started, "render_started")?;
        let render_completed = required(timestamps.render_completed, "render_completed")?;
        Ok(ClientVideoFrameTimings {
            decoder_queue: elapsed(timestamps.decoder_submitted, decoder_entered),
            decode: elapsed(decoder_entered, decoder_completed),
            presentation_queue: elapsed(decoder_completed, gui_received),
            render_queue: elapsed(gui_received, render_started),
            render: elapsed(render_started, render_completed),
            client_latency: elapsed(timestamps.decoder_submitted, render_completed),
        })
    }
}

impl ClientVideoProbeReporter {
    pub(crate) fn new(report_interval: Duration) -> Self {
        Self {
            report_interval,
            window_started: None,
            frames: 0,
            rtp_packets: 0,
            rtp_bytes: 0,
            totals: ClientVideoProbeStageTotals::default(),
        }
    }

    pub(crate) fn record_frame(&mut self, screen_id: ScreenId, probe: ClientVideoFrameProbe) {
        let now = Instant::now();
        let timings = match probe.finish() {
            Ok(timings) => timings,
            Err(stage) => {
                tracing::warn!(
                    target: "rabbit::client_video_probe",
                    platform = "windows",
                    screen_id = screen_id.get(),
                    frame_id = probe.frame_id,
                    missing_stage = stage,
                    "Client video frame probe is incomplete"
                );
                return;
            }
        };

        tracing::trace!(
            target: "rabbit::client_video_probe",
            platform = "windows",
            screen_id = screen_id.get(),
            frame_id = probe.frame_id,
            decoder_queue_ms = %TwoDecimal(duration_ms(timings.decoder_queue)),
            decode_ms = %TwoDecimal(duration_ms(timings.decode)),
            presentation_queue_ms = %TwoDecimal(duration_ms(timings.presentation_queue)),
            render_queue_ms = %TwoDecimal(duration_ms(timings.render_queue)),
            render_ms = %TwoDecimal(duration_ms(timings.render)),
            client_latency_ms = %TwoDecimal(duration_ms(timings.client_latency)),
            rtp_packets = probe.rtp_packets,
            rtp_bytes = probe.rtp_bytes,
            "Client video frame rendered"
        );

        self.window_started.get_or_insert(now);
        self.frames = self.frames.saturating_add(1);
        self.rtp_packets = self.rtp_packets.saturating_add(probe.rtp_packets);
        self.rtp_bytes = self.rtp_bytes.saturating_add(probe.rtp_bytes);
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
            target: "rabbit::client_video_probe",
            platform = "windows",
            partial,
            window_ms = %TwoDecimal(duration_ms(window_elapsed)),
            frames,
            fps = %TwoDecimal(rate(frames, window_elapsed)),
            avg_client_latency_ms = %TwoDecimal(average_ms(self.totals.client_latency, frames)),
            avg_decoder_queue_ms = %TwoDecimal(average_ms(self.totals.decoder_queue, frames)),
            avg_decode_ms = %TwoDecimal(average_ms(self.totals.decode, frames)),
            avg_presentation_queue_ms = %TwoDecimal(
                average_ms(self.totals.presentation_queue, frames)
            ),
            avg_render_queue_ms = %TwoDecimal(average_ms(self.totals.render_queue, frames)),
            avg_render_ms = %TwoDecimal(average_ms(self.totals.render, frames)),
            rtp_packets = self.rtp_packets,
            rtp_bytes = self.rtp_bytes,
            "Client video throughput window"
        );

        self.window_started = Some(now);
        self.frames = 0;
        self.rtp_packets = 0;
        self.rtp_bytes = 0;
        self.totals = ClientVideoProbeStageTotals::default();
    }
}

impl ClientVideoProbeStageTotals {
    fn add(&mut self, timings: &ClientVideoFrameTimings) {
        self.decoder_queue += timings.decoder_queue;
        self.decode += timings.decode;
        self.presentation_queue += timings.presentation_queue;
        self.render_queue += timings.render_queue;
        self.render += timings.render;
        self.client_latency += timings.client_latency;
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
