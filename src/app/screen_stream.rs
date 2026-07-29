use std::{
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
        session::SessionSend,
        transport::TransportSend,
        video_encoder::{VideoEncoder, VideoEncoderCommand, VideoEncoderParameters},
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
    let Some(max_packet_size) = session.max_video_packet_size() else {
        eros::bail!(
            "Session transport does not support video datagrams for screen {}",
            screen_id.0
        );
    };

    info!(
        event = "host_screen_stream_started",
        screen_id = screen_id.0,
        codec = ?parameters.codec,
        frame_rate_numerator = parameters.frame_rate.numerator(),
        frame_rate_denominator = parameters.frame_rate.denominator(),
        frame_rate_mode = ?parameters.frame_rate_mode,
        bitrate_bps = parameters.bitrate.bits_per_second(),
        max_packet_size,
        "Host screen stream started"
    );

    let commands = futures_util::stream::poll_fn(move |context| {
        let mut command = encoder_commands.pop();
        Pin::new(&mut command).poll(context).map(Some)
    });
    let mut scheduler = VideoDatagramScheduler::new(parameters);

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
            let scheduled = scheduler.schedule_access_unit(payloads);
            async move { scheduled.send(&session, screen_id).await }
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

    fn schedule_access_unit(&mut self, payloads: Vec<bytes::Bytes>) -> ScheduledVideoAccessUnit {
        let started_at = Instant::now();
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
        }
    }
}

struct ScheduledVideoAccessUnit {
    started_at: Instant,
    maximum_age: Duration,
    batches: Vec<(Instant, Vec<bytes::Bytes>)>,
}

impl ScheduledVideoAccessUnit {
    async fn send<Send>(self, session: &SessionSend<Send>, screen_id: ScreenId) -> eros::Result<()>
    where
        Send: TransportSend,
    {
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
        Ok(())
    }
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
    use std::{collections::VecDeque, future::poll_fn, pin::Pin};

    use bytes::Bytes;
    use futures_core::Stream as _;

    use crate::{
        app::screen_stream::{
            CancellableFrames, VIDEO_BATCH_MAX_BYTES, VIDEO_BATCH_MAX_PACKETS,
            take_video_datagram_batch,
        },
        infra::unsync_queue::UnsyncQueue,
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
}
