use std::{
    fmt::{self, Display, Formatter},
    time::{Duration, Instant},
};

use gstreamer::prelude::{ElementExt as _, PadExtManual as _};

use crate::kernel::screen_manager::ScreenId;

#[derive(Debug, Default)]
pub(crate) struct ClientVideoProbeClock {
    next_frame_id: u64,
}

#[derive(Debug)]
pub(crate) struct ClientVideoDecodeProbe {
    clock: ClientVideoProbeClock,
    submitted: flume::Sender<ClientVideoFrameProbe>,
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
    dma_buf_import_started: Option<Instant>,
    dma_buf_import_completed: Option<Instant>,
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
    dma_buf_import_queue: Duration,
    dma_buf_import: Duration,
    render_queue: Duration,
    render: Duration,
    client_latency: Duration,
}

struct ClientVideoFrameTimings {
    decoder_queue: Duration,
    decode: Duration,
    presentation_queue: Duration,
    dma_buf_import_queue: Duration,
    dma_buf_import: Duration,
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

        ClientVideoFrameProbe::new(
            frame_id,
            u64::try_from(rtp_packets).unwrap_or(u64::MAX),
            u64::try_from(rtp_bytes).unwrap_or(u64::MAX),
            Instant::now(),
        )
    }
}

impl ClientVideoDecodeProbe {
    pub(crate) fn new(
        decoder: &gstreamer::Element,
    ) -> eros::Result<(Self, flume::Receiver<ClientVideoFrameProbe>)> {
        let (submitted, decoder_inputs) = flume::unbounded::<ClientVideoFrameProbe>();
        let (decoder_entered, entered_frames) = flume::unbounded::<ClientVideoFrameProbe>();
        let (decoder_completed, completed_frames) = flume::unbounded::<ClientVideoFrameProbe>();

        let Some(sink_pad) = decoder.static_pad("sink") else {
            eros::bail!("GStreamer H.264 decoder does not expose a sink pad");
        };
        sink_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, _| {
            let Ok(mut probe) = decoder_inputs.try_recv() else {
                tracing::warn!(
                    target: "rabbit::client_video_probe",
                    "Hardware decoder received a frame without a client video probe"
                );
                return gstreamer::PadProbeReturn::Ok;
            };
            probe.mark_decoder_entered();
            if decoder_entered.send(probe).is_err() {
                tracing::warn!(
                    target: "rabbit::client_video_probe",
                    "Client video decoder-entered probe channel disconnected"
                );
            }
            gstreamer::PadProbeReturn::Ok
        });

        let Some(source_pad) = decoder.static_pad("src") else {
            eros::bail!("GStreamer H.264 decoder does not expose a source pad");
        };
        source_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |_, _| {
            let Ok(mut probe) = entered_frames.try_recv() else {
                tracing::warn!(
                    target: "rabbit::client_video_probe",
                    "Hardware decoder produced a frame without an entered client video probe"
                );
                return gstreamer::PadProbeReturn::Ok;
            };
            probe.mark_decoder_completed();
            if decoder_completed.send(probe).is_err() {
                tracing::warn!(
                    target: "rabbit::client_video_probe",
                    "Client video decoder-completed probe channel disconnected"
                );
            }
            gstreamer::PadProbeReturn::Ok
        });

        Ok((
            Self {
                clock: ClientVideoProbeClock::default(),
                submitted,
            },
            completed_frames,
        ))
    }

    pub(crate) fn submit_frame(
        &mut self,
        rtp_packets: usize,
        rtp_bytes: usize,
    ) -> eros::Result<()> {
        let probe = self.clock.frame(rtp_packets, rtp_bytes);
        if self.submitted.send(probe).is_err() {
            eros::bail!("Client video decoder probe channel disconnected");
        }
        Ok(())
    }
}

impl ClientVideoFrameProbe {
    fn new(frame_id: u64, rtp_packets: u64, rtp_bytes: u64, submitted: Instant) -> Self {
        Self {
            frame_id,
            rtp_packets,
            rtp_bytes,
            timestamps: ClientVideoFrameTimestamps {
                decoder_submitted: submitted,
                decoder_entered: None,
                decoder_completed: None,
                gui_received: None,
                dma_buf_import_started: None,
                dma_buf_import_completed: None,
                render_started: None,
                render_completed: None,
            },
        }
    }

    pub(crate) fn mark_decoder_entered(&mut self) {
        self.timestamps.decoder_entered = Some(Instant::now());
    }

    pub(crate) fn mark_decoder_completed(&mut self) {
        self.timestamps.decoder_completed = Some(Instant::now());
    }

    pub(crate) fn mark_gui_received(&mut self) {
        self.timestamps.gui_received = Some(Instant::now());
    }

    pub(crate) fn mark_dma_buf_import_started(&mut self) {
        self.timestamps.dma_buf_import_started = Some(Instant::now());
    }

    pub(crate) fn mark_dma_buf_import_completed(&mut self) {
        self.timestamps.dma_buf_import_completed = Some(Instant::now());
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
        let dma_buf_import_started =
            required(timestamps.dma_buf_import_started, "dma_buf_import_started")?;
        let dma_buf_import_completed = required(
            timestamps.dma_buf_import_completed,
            "dma_buf_import_completed",
        )?;
        let render_started = required(timestamps.render_started, "render_started")?;
        let render_completed = required(timestamps.render_completed, "render_completed")?;

        Ok(ClientVideoFrameTimings {
            decoder_queue: elapsed(timestamps.decoder_submitted, decoder_entered),
            decode: elapsed(decoder_entered, decoder_completed),
            presentation_queue: elapsed(decoder_completed, gui_received),
            dma_buf_import_queue: elapsed(gui_received, dma_buf_import_started),
            dma_buf_import: elapsed(dma_buf_import_started, dma_buf_import_completed),
            render_queue: elapsed(dma_buf_import_completed, render_started),
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
            screen_id = screen_id.get(),
            frame_id = probe.frame_id,
            decoder_queue_ms = %TwoDecimal(duration_ms(timings.decoder_queue)),
            decode_ms = %TwoDecimal(duration_ms(timings.decode)),
            presentation_queue_ms = %TwoDecimal(duration_ms(timings.presentation_queue)),
            dma_buf_import_queue_ms = %TwoDecimal(duration_ms(timings.dma_buf_import_queue)),
            dma_buf_import_ms = %TwoDecimal(duration_ms(timings.dma_buf_import)),
            render_queue_ms = %TwoDecimal(duration_ms(timings.render_queue)),
            render_ms = %TwoDecimal(duration_ms(timings.render)),
            client_latency_ms = %TwoDecimal(duration_ms(timings.client_latency)),
            rtp_packets = probe.rtp_packets,
            rtp_bytes = probe.rtp_bytes,
            "Client video frame rendered"
        );

        self.window_started.get_or_insert(now);
        self.frames += 1;
        self.rtp_packets += probe.rtp_packets;
        self.rtp_bytes += probe.rtp_bytes;
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
            partial,
            window_ms = %TwoDecimal(duration_ms(window_elapsed)),
            frames,
            fps = %TwoDecimal(rate(frames, window_elapsed)),
            avg_client_latency_ms = %TwoDecimal(average_ms(self.totals.client_latency, frames)),
            avg_decoder_queue_ms = %TwoDecimal(average_ms(self.totals.decoder_queue, frames)),
            avg_decode_ms = %TwoDecimal(average_ms(self.totals.decode, frames)),
            avg_presentation_queue_ms = %TwoDecimal(average_ms(self.totals.presentation_queue, frames)),
            avg_dma_buf_import_queue_ms = %TwoDecimal(average_ms(
                self.totals.dma_buf_import_queue,
                frames
            )),
            avg_dma_buf_import_ms = %TwoDecimal(average_ms(self.totals.dma_buf_import, frames)),
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
        self.dma_buf_import_queue += timings.dma_buf_import_queue;
        self.dma_buf_import += timings.dma_buf_import;
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

// Focused test: cargo test infra::platform::client_video_probe::tests:: --lib
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gstreamer::glib::prelude::Cast as _;
    use gstreamer::prelude::{ElementExt as _, GstBinExtManual as _};

    use crate::{
        infra::platform::client_video_probe::{
            ClientVideoDecodeProbe, ClientVideoFrameProbe, ClientVideoProbeClock,
            ClientVideoProbeReporter, TwoDecimal,
        },
        kernel::screen_manager::ScreenId,
    };

    #[test]
    fn assigns_monotonic_client_frame_ids() {
        let mut clock = ClientVideoProbeClock::default();

        let first = clock.frame(2, 1_200);
        let second = clock.frame(3, 1_800);

        assert_eq!(first.frame_id, 0);
        assert_eq!(second.frame_id, 1);
        assert_eq!(second.rtp_packets, 3);
        assert_eq!(second.rtp_bytes, 1_800);
    }

    #[test]
    fn moves_a_client_probe_through_decoder_pads() {
        gstreamer::init().expect("GStreamer should initialize for the client decode probe test");
        let pipeline = gstreamer::Pipeline::new();
        let source = gstreamer::ElementFactory::make("appsrc")
            .build()
            .expect("The client decode probe test should create appsrc")
            .downcast::<gstreamer_app::AppSrc>()
            .expect("The client decode probe test source should be AppSrc");
        let decoder = gstreamer::ElementFactory::make("identity")
            .build()
            .expect("The client decode probe test should create an identity decoder");
        let sink = gstreamer::ElementFactory::make("appsink")
            .build()
            .expect("The client decode probe test should create appsink")
            .downcast::<gstreamer_app::AppSink>()
            .expect("The client decode probe test sink should be AppSink");
        sink.set_sync(false);
        sink.set_async(false);
        let elements = [source.upcast_ref(), &decoder, sink.upcast_ref()];
        pipeline
            .add_many(elements)
            .expect("The client decode probe test elements should join one pipeline");
        gstreamer::Element::link_many(elements)
            .expect("The client decode probe test elements should link");
        let (mut probe, completed) = ClientVideoDecodeProbe::new(&decoder)
            .expect("The client decode probe should attach to decoder pads");
        pipeline
            .set_state(gstreamer::State::Playing)
            .expect("The client decode probe test pipeline should start");

        probe
            .submit_frame(2, 1_200)
            .expect("The encoded frame probe should reach the decoder");
        source
            .push_buffer(gstreamer::Buffer::from_slice([0_u8]))
            .expect("The client decode probe test buffer should reach the decoder");
        sink.pull_sample()
            .expect("The client decode probe test pipeline should produce a sample");
        let completed = completed
            .recv_timeout(Duration::from_secs(1))
            .expect("The client decode probe should leave the decoder with its frame");

        assert_eq!(completed.frame_id(), 0);
        assert!(completed.timestamps.decoder_entered.is_some());
        assert!(completed.timestamps.decoder_completed.is_some());
        pipeline
            .set_state(gstreamer::State::Null)
            .expect("The client decode probe test pipeline should stop");
    }

    #[test]
    fn calculates_complete_client_video_stage_timings() {
        let start = Instant::now();
        let probe = completed_probe(start);

        let timings = probe
            .finish()
            .expect("A complete client video probe should produce timings");

        assert_eq!(timings.decoder_queue, Duration::from_millis(2));
        assert_eq!(timings.decode, Duration::from_millis(3));
        assert_eq!(timings.presentation_queue, Duration::from_millis(5));
        assert_eq!(timings.dma_buf_import_queue, Duration::from_millis(7));
        assert_eq!(timings.dma_buf_import, Duration::from_millis(11));
        assert_eq!(timings.render_queue, Duration::from_millis(13));
        assert_eq!(timings.render, Duration::from_millis(17));
        assert_eq!(timings.client_latency, Duration::from_millis(58));
    }

    #[test]
    fn rejects_an_incomplete_client_video_probe() {
        let probe = ClientVideoFrameProbe::new(0, 1, 600, Instant::now());

        assert_eq!(probe.finish().err(), Some("decoder_entered"));
    }

    #[test]
    fn reporter_accumulates_completed_client_frames() {
        let start = Instant::now() - Duration::from_millis(100);
        let mut reporter = ClientVideoProbeReporter::new(Duration::from_secs(2));

        reporter.record_frame(ScreenId(2), completed_probe(start));

        assert_eq!(reporter.frames, 1);
        assert_eq!(reporter.rtp_packets, 3);
        assert_eq!(reporter.rtp_bytes, 1_800);
        assert_eq!(reporter.totals.decode, Duration::from_millis(3));
        assert_eq!(reporter.totals.client_latency, Duration::from_millis(58));
    }

    #[test]
    fn finishing_a_client_video_probe_window_resets_its_start() {
        let start = Instant::now() - Duration::from_millis(100);
        let mut reporter = ClientVideoProbeReporter::new(Duration::from_secs(2));
        reporter.record_frame(ScreenId(2), completed_probe(start));

        reporter.finish();

        assert!(reporter.window_started.is_none());
        assert_eq!(reporter.frames, 0);
    }

    #[test]
    fn reports_after_the_configured_client_video_interval() {
        let start = Instant::now() - Duration::from_millis(100);
        let mut reporter = ClientVideoProbeReporter::new(Duration::from_millis(50));
        reporter.record_frame(ScreenId(2), completed_probe(start));
        reporter.window_started = Some(Instant::now() - Duration::from_millis(100));

        reporter.record_frame(ScreenId(2), completed_probe(start));

        assert_eq!(reporter.frames, 0);
    }

    fn completed_probe(start: Instant) -> ClientVideoFrameProbe {
        let mut probe = ClientVideoFrameProbe::new(7, 3, 1_800, start);
        probe.timestamps.decoder_entered = Some(start + Duration::from_millis(2));
        probe.timestamps.decoder_completed = Some(start + Duration::from_millis(5));
        probe.timestamps.gui_received = Some(start + Duration::from_millis(10));
        probe.timestamps.dma_buf_import_started = Some(start + Duration::from_millis(17));
        probe.timestamps.dma_buf_import_completed = Some(start + Duration::from_millis(28));
        probe.timestamps.render_started = Some(start + Duration::from_millis(41));
        probe.timestamps.render_completed = Some(start + Duration::from_millis(58));
        probe
    }

    #[test]
    fn formats_client_video_floats_with_two_decimal_places() {
        assert_eq!(TwoDecimal(115.12956516018339).to_string(), "115.13");
    }
}
