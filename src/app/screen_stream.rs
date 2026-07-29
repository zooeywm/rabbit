use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future as _,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use eros::Context as _;
use tracing::{info, trace};

use crate::{
    infra::unsync_queue::UnsyncQueue,
    kernel::{
        screen_manager::ScreenId,
        screen_stream::ScreenStream,
        session::{
            SessionSend,
            fec::{encode_access_unit, fec_rtp_packet_size},
        },
        transport::{TransportSend, TransportTelemetry},
        video_encoder::{VideoBitrate, VideoEncoder, VideoEncoderCommand, VideoEncoderParameters},
    },
};

pub(crate) async fn run_host_screen_stream<Frames, Send, Encoder>(
    frames: Frames,
    screen_id: ScreenId,
    session: Rc<SessionSend<Send>>,
    cancellation: UnsyncQueue<()>,
    encoder_commands: UnsyncQueue<VideoEncoderCommand>,
    parameters: VideoEncoderParameters,
) -> eros::Result<()>
where
    Encoder: VideoEncoder,
    Encoder::Packet: Into<bytes::Bytes>,
    Frames: futures_core::Stream<Item = eros::Result<Rc<Encoder::Input>>> + Unpin,
    Send: TransportSend,
{
    let Some(max_datagram_size) = session.max_video_packet_size() else {
        eros::bail!(
            "Session transport does not support video datagrams for screen {}",
            screen_id.0
        );
    };
    let max_packet_size = fec_rtp_packet_size(max_datagram_size, parameters.fec_percentage)?;

    info!(
        event = "host_screen_stream_started",
        screen_id = screen_id.0,
        codec = ?parameters.codec,
        frame_rate_numerator = parameters.frame_rate.numerator(),
        frame_rate_denominator = parameters.frame_rate.denominator(),
        frame_rate_mode = ?parameters.frame_rate_mode,
        bitrate_bps = parameters.bitrate.bits_per_second(),
        fec_percentage = parameters.fec_percentage.get(),
        max_datagram_size,
        max_packet_size,
        "Host screen stream started"
    );

    let adaptive_commands = encoder_commands.clone();
    let commands = futures_util::stream::poll_fn(move |context| {
        let mut command = encoder_commands.pop();
        Pin::new(&mut command).poll(context).map(Some)
    });
    let mut scheduler = VideoDatagramScheduler::new(parameters);
    let bitrate_controller = Rc::new(RefCell::new(AdaptiveBitrateController::new(parameters)));

    ScreenStream::<_, _, Encoder, _>::new(
        CancellableFrames {
            frames,
            cancellation,
        },
        commands,
        parameters,
        max_packet_size,
        move |packets: Vec<Encoder::Packet>| {
            let session = Rc::clone(&session);
            let payloads = packets
                .into_iter()
                .map(Into::into)
                .collect::<Vec<bytes::Bytes>>();
            let rtp_packets = payloads.len();
            let rtp_bytes = payloads.iter().map(bytes::Bytes::len).sum();
            let h264_bytes = payloads
                .iter()
                .map(|packet| packet.len().saturating_sub(RTP_FIXED_HEADER_SIZE))
                .sum();
            let scheduled =
                encode_access_unit(payloads, parameters.fec_percentage).map(|payloads| {
                    scheduler.schedule_access_unit(payloads, h264_bytes, rtp_packets, rtp_bytes)
                });
            let bitrate_controller = Rc::clone(&bitrate_controller);
            let adaptive_commands = adaptive_commands.clone();
            async move {
                let report = scheduled?.send(&session, screen_id).await?;
                if let Some(bitrate) = bitrate_controller.borrow_mut().record(report) {
                    adaptive_commands.push(VideoEncoderCommand::SetBitrate(bitrate));
                }
                Ok(())
            }
        },
    )
    .run()
    .await
    .with_context(|| format!("Failed to stream screen {}", screen_id.0))
}

struct VideoDatagramScheduler {
    next_batch_at: Option<Instant>,
    batch_interval: Duration,
    max_access_unit_age: Duration,
}

impl VideoDatagramScheduler {
    fn new(parameters: VideoEncoderParameters) -> Self {
        let numerator = u128::from(parameters.frame_rate.numerator().max(1));
        let denominator = u128::from(parameters.frame_rate.denominator().max(1));
        let frame_nanoseconds = 1_000_000_000_u128
            .saturating_mul(denominator)
            .div_ceil(numerator)
            .min(u64::MAX.into()) as u64;
        Self {
            next_batch_at: None,
            batch_interval: VIDEO_BATCH_INTERVAL,
            max_access_unit_age: Duration::from_nanos(frame_nanoseconds).saturating_mul(2),
        }
    }

    fn schedule_access_unit(
        &mut self,
        payloads: Vec<bytes::Bytes>,
        h264_bytes: usize,
        rtp_packets: usize,
        rtp_bytes: usize,
    ) -> ScheduledVideoAccessUnit {
        let started_at = Instant::now();
        let fec_bytes = payloads.iter().map(bytes::Bytes::len).sum();
        let mut pending = VecDeque::from(payloads);
        let mut send_at = self.next_batch_at.unwrap_or(started_at).max(started_at);
        let mut batches = Vec::new();
        while !pending.is_empty() {
            let batch = take_video_datagram_batch(&mut pending);
            batches.push((send_at, batch));
            send_at = send_at.checked_add(self.batch_interval).unwrap_or(send_at);
        }
        if !batches.is_empty() {
            self.next_batch_at = Some(send_at);
        }
        ScheduledVideoAccessUnit {
            started_at,
            maximum_age: self.max_access_unit_age,
            batches,
            h264_bytes,
            rtp_packets,
            rtp_bytes,
            fec_bytes,
        }
    }
}

struct ScheduledVideoAccessUnit {
    started_at: Instant,
    maximum_age: Duration,
    batches: Vec<(Instant, Vec<bytes::Bytes>)>,
    h264_bytes: usize,
    rtp_packets: usize,
    rtp_bytes: usize,
    fec_bytes: usize,
}

impl ScheduledVideoAccessUnit {
    async fn send<Send>(
        self,
        session: &SessionSend<Send>,
        screen_id: ScreenId,
    ) -> eros::Result<VideoSendReport>
    where
        Send: TransportSend,
    {
        let telemetry_before = session.transport_telemetry();
        let batch_count = self.batches.len();
        for (batch_index, (send_at, batch)) in self.batches.into_iter().enumerate() {
            let delay = send_at.saturating_duration_since(Instant::now());
            if !delay.is_zero() {
                compio::time::sleep(delay).await;
            }
            let payload_size = batch.iter().map(bytes::Bytes::len).sum::<usize>();
            trace!(
                event = "video_packet_batch_send",
                screen_id = screen_id.0,
                batch_index,
                batch_count,
                packet_count = batch.len(),
                payload_size,
                "Sending paced video datagram batch"
            );
            session.send_video_batch(screen_id, batch).await?;
        }
        let access_unit_age = self.started_at.elapsed();
        if access_unit_age > self.maximum_age {
            trace!(
                event = "video_access_unit_send_late",
                screen_id = screen_id.0,
                access_unit_age_ms = access_unit_age.as_secs_f64() * 1_000.0,
                maximum_age_ms = self.maximum_age.as_secs_f64() * 1_000.0,
                "A complete video access unit exceeded its pacing age budget"
            );
        }
        Ok(VideoSendReport {
            elapsed: access_unit_age,
            maximum_age: self.maximum_age,
            h264_bytes: self.h264_bytes,
            rtp_packets: self.rtp_packets,
            rtp_bytes: self.rtp_bytes,
            fec_bytes: self.fec_bytes,
            telemetry_before,
            telemetry_after: session.transport_telemetry(),
        })
    }
}

struct VideoSendReport {
    elapsed: Duration,
    maximum_age: Duration,
    h264_bytes: usize,
    rtp_packets: usize,
    rtp_bytes: usize,
    fec_bytes: usize,
    telemetry_before: Option<TransportTelemetry>,
    telemetry_after: Option<TransportTelemetry>,
}

struct AdaptiveBitrateController {
    requested: VideoBitrate,
    current: VideoBitrate,
    minimum_bps: u32,
    window_started: Instant,
    last_telemetry: Option<TransportTelemetry>,
    minimum_rtt: Option<Duration>,
    stable_windows: u8,
    frames: u64,
    h264_bytes: u64,
    rtp_packets: u64,
    rtp_bytes: u64,
    fec_bytes: u64,
    late_frames: u64,
}

impl AdaptiveBitrateController {
    fn new(parameters: VideoEncoderParameters) -> Self {
        let requested_bps = parameters.bitrate.bits_per_second();
        Self {
            requested: parameters.bitrate,
            current: parameters.bitrate,
            minimum_bps: (requested_bps / 4).max(MIN_ADAPTIVE_BITRATE_BPS),
            window_started: Instant::now(),
            last_telemetry: None,
            minimum_rtt: None,
            stable_windows: 0,
            frames: 0,
            h264_bytes: 0,
            rtp_packets: 0,
            rtp_bytes: 0,
            fec_bytes: 0,
            late_frames: 0,
        }
    }

    fn record(&mut self, report: VideoSendReport) -> Option<VideoBitrate> {
        self.frames = self.frames.saturating_add(1);
        self.h264_bytes = self.h264_bytes.saturating_add(report.h264_bytes as u64);
        self.rtp_packets = self.rtp_packets.saturating_add(report.rtp_packets as u64);
        self.rtp_bytes = self.rtp_bytes.saturating_add(report.rtp_bytes as u64);
        self.fec_bytes = self.fec_bytes.saturating_add(report.fec_bytes as u64);
        self.late_frames = self
            .late_frames
            .saturating_add(u64::from(report.elapsed > report.maximum_age));
        let telemetry = report.telemetry_after.or(report.telemetry_before);
        if let Some(telemetry) = telemetry {
            self.minimum_rtt = Some(
                self.minimum_rtt
                    .map_or(telemetry.rtt, |minimum| minimum.min(telemetry.rtt)),
            );
        }
        let elapsed = self.window_started.elapsed();
        if elapsed < ADAPTIVE_BITRATE_WINDOW {
            return None;
        }

        let previous = self.last_telemetry;
        self.last_telemetry = telemetry;
        let lost_packets = counter_delta(telemetry, previous, |value| value.lost_packets);
        let congestion_events = counter_delta(telemetry, previous, |value| value.congestion_events);
        let quic_bytes = counter_delta(telemetry, previous, |value| value.transmitted_bytes);
        let rtt_inflated = telemetry
            .zip(self.minimum_rtt)
            .is_some_and(|(current, minimum)| {
                current.rtt
                    > minimum
                        .saturating_mul(3)
                        .checked_div(2)
                        .unwrap_or(minimum)
                        .saturating_add(Duration::from_millis(5))
            });
        let datagram_buffer_exhausted =
            telemetry.is_some_and(|value| value.datagram_buffer_space == 0);
        let congested = lost_packets > 0
            || congestion_events > 0
            || rtt_inflated
            || datagram_buffer_exhausted
            || self.late_frames > 0;
        let old_bitrate = self.current;
        if congested {
            self.stable_windows = 0;
            self.current = VideoBitrate::new(
                self.current
                    .bits_per_second()
                    .saturating_mul(85)
                    .checked_div(100)
                    .unwrap_or(self.minimum_bps)
                    .max(self.minimum_bps),
            )
            .expect("adaptive bitrate minimum is positive");
        } else {
            self.stable_windows = self.stable_windows.saturating_add(1);
            if self.stable_windows >= ADAPTIVE_STABLE_WINDOWS {
                self.stable_windows = 0;
                self.current = VideoBitrate::new(
                    self.current
                        .bits_per_second()
                        .saturating_add(ADAPTIVE_BITRATE_INCREMENT_BPS)
                        .min(self.requested.bits_per_second()),
                )
                .expect("requested bitrate is positive");
            }
        }

        info!(
            event = "video_bitrate_feedback",
            window_ms = elapsed.as_secs_f64() * 1_000.0,
            requested_bitrate_bps = self.requested.bits_per_second(),
            target_bitrate_bps = self.current.bits_per_second(),
            frames = self.frames,
            h264_bytes = self.h264_bytes,
            h264_bps = byte_rate_bps(self.h264_bytes, elapsed),
            rtp_packets = self.rtp_packets,
            rtp_bytes = self.rtp_bytes,
            rtp_bps = byte_rate_bps(self.rtp_bytes, elapsed),
            fec_bytes = self.fec_bytes,
            fec_bps = byte_rate_bps(self.fec_bytes, elapsed),
            quic_bytes,
            quic_bps = byte_rate_bps(quic_bytes, elapsed),
            lost_packets,
            congestion_events,
            rtt_ms = telemetry.map_or(0.0, |value| value.rtt.as_secs_f64() * 1_000.0),
            congestion_window = telemetry.map_or(0, |value| value.congestion_window),
            datagram_buffer_space = telemetry.map_or(0, |value| value.datagram_buffer_space),
            late_frames = self.late_frames,
            "Video bitrate feedback window"
        );
        self.window_started = Instant::now();
        self.frames = 0;
        self.h264_bytes = 0;
        self.rtp_packets = 0;
        self.rtp_bytes = 0;
        self.fec_bytes = 0;
        self.late_frames = 0;
        (self.current != old_bitrate).then_some(self.current)
    }
}

fn counter_delta(
    current: Option<TransportTelemetry>,
    previous: Option<TransportTelemetry>,
    select: impl Fn(TransportTelemetry) -> u64,
) -> u64 {
    match (current, previous) {
        (Some(current), Some(previous)) => select(current).saturating_sub(select(previous)),
        _ => 0,
    }
}

fn byte_rate_bps(bytes: u64, elapsed: Duration) -> u64 {
    let nanoseconds = elapsed.as_nanos().max(1);
    u64::try_from(
        u128::from(bytes)
            .saturating_mul(8_000_000_000)
            .checked_div(nanoseconds)
            .unwrap_or(0)
            .min(u128::from(u64::MAX)),
    )
    .unwrap_or(u64::MAX)
}

fn take_video_datagram_batch(pending: &mut VecDeque<bytes::Bytes>) -> Vec<bytes::Bytes> {
    let mut bytes = 0usize;
    let mut batch = Vec::with_capacity(VIDEO_BATCH_MAX_PACKETS.min(pending.len()));
    while batch.len() < VIDEO_BATCH_MAX_PACKETS {
        let Some(next) = pending.front() else {
            break;
        };
        if !batch.is_empty() && bytes.saturating_add(next.len()) > VIDEO_BATCH_MAX_BYTES {
            break;
        }
        let packet = pending
            .pop_front()
            .expect("the video datagram queue front was present");
        bytes = bytes.saturating_add(packet.len());
        batch.push(packet);
    }
    batch
}

const VIDEO_BATCH_MAX_PACKETS: usize = 64;
const VIDEO_BATCH_MAX_BYTES: usize = 64 * 1024;
const VIDEO_BATCH_INTERVAL: Duration = Duration::from_micros(750);
const RTP_FIXED_HEADER_SIZE: usize = 12;
const MIN_ADAPTIVE_BITRATE_BPS: u32 = 2_000_000;
const ADAPTIVE_BITRATE_WINDOW: Duration = Duration::from_secs(1);
const ADAPTIVE_STABLE_WINDOWS: u8 = 4;
const ADAPTIVE_BITRATE_INCREMENT_BPS: u32 = 500_000;

struct CancellableFrames<Frames> {
    frames: Frames,
    cancellation: UnsyncQueue<()>,
}

impl<Frames> futures_core::Stream for CancellableFrames<Frames>
where
    Frames: futures_core::Stream + Unpin,
{
    type Item = Frames::Item;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        let cancelled = {
            let mut cancellation = this.cancellation.pop();
            Pin::new(&mut cancellation).poll(context).is_ready()
        };
        if cancelled {
            return Poll::Ready(None);
        }

        Pin::new(&mut this.frames).poll_next(context)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        future::poll_fn,
        pin::Pin,
        time::{Duration, Instant},
    };

    use bytes::Bytes;
    use futures_core::Stream as _;

    use crate::{
        app::screen_stream::{
            AdaptiveBitrateController, CancellableFrames, VIDEO_BATCH_MAX_BYTES,
            VIDEO_BATCH_MAX_PACKETS, VideoSendReport, take_video_datagram_batch,
        },
        infra::unsync_queue::UnsyncQueue,
        kernel::{
            geometry::FrameRate,
            transport::TransportTelemetry,
            video_encoder::{
                VideoBitrate, VideoCodec, VideoEncoderParameters, VideoFecPercentage,
                VideoFrameRateMode,
            },
        },
    };

    #[test]
    fn cancellation_closes_frame_stream() {
        let cancellation = UnsyncQueue::default();
        let mut frames = CancellableFrames {
            frames: futures_util::stream::pending::<u8>(),
            cancellation: cancellation.clone(),
        };
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        cancellation.push(());

        assert!(
            runtime
                .block_on(poll_fn(|context| Pin::new(&mut frames).poll_next(context)))
                .is_none()
        );
    }

    #[test]
    fn video_datagram_batch_respects_packet_and_byte_limits() {
        let mut byte_limited = VecDeque::from([
            Bytes::from(vec![0; 40_000]),
            Bytes::from(vec![0; 30_000]),
            Bytes::from_static(&[1]),
        ]);
        let first = take_video_datagram_batch(&mut byte_limited);
        assert_eq!(first.len(), 1);
        assert!(first.iter().map(Bytes::len).sum::<usize>() <= VIDEO_BATCH_MAX_BYTES);

        let mut packet_limited =
            VecDeque::from(vec![Bytes::from_static(&[1]); VIDEO_BATCH_MAX_PACKETS + 1]);
        let first = take_video_datagram_batch(&mut packet_limited);
        assert_eq!(first.len(), VIDEO_BATCH_MAX_PACKETS);
        assert_eq!(packet_limited.len(), 1);
    }

    #[test]
    fn adaptive_bitrate_reduces_on_quic_loss() {
        let parameters = VideoEncoderParameters {
            codec: VideoCodec::H264,
            frame_rate: FrameRate::new(120, 1).expect("frame rate"),
            frame_rate_mode: VideoFrameRateMode::Dynamic,
            bitrate: VideoBitrate::new(40_000_000).expect("bitrate"),
            fec_percentage: VideoFecPercentage::DEFAULT,
        };
        let mut controller = AdaptiveBitrateController::new(parameters);
        controller.window_started = Instant::now() - Duration::from_secs(2);
        controller.last_telemetry = Some(telemetry(100, 0));

        let adjusted = controller.record(VideoSendReport {
            elapsed: Duration::from_millis(2),
            maximum_age: Duration::from_millis(16),
            h264_bytes: 10_000,
            rtp_packets: 10,
            rtp_bytes: 10_120,
            fec_bytes: 12_000,
            telemetry_before: None,
            telemetry_after: Some(telemetry(200, 1)),
        });

        assert_eq!(
            adjusted.map(VideoBitrate::bits_per_second),
            Some(34_000_000)
        );
    }

    fn telemetry(sent_packets: u64, lost_packets: u64) -> TransportTelemetry {
        TransportTelemetry {
            rtt: Duration::from_millis(2),
            congestion_window: 1_000_000,
            congestion_events: lost_packets,
            lost_packets,
            sent_packets,
            transmitted_bytes: sent_packets.saturating_mul(1_200),
            datagram_buffer_space: 64 * 1024,
        }
    }
}
