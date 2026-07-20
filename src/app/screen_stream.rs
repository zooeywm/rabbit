use std::{
    future::Future as _,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use eros::Context as _;

use crate::{
    infra::unsync_queue::UnsyncQueue,
    kernel::{
        screen_manager::ScreenId,
        screen_stream::ScreenStream,
        session::{SessionSend, VideoMessage},
        transport::TransportSend,
        video_encoder::VideoEncoder,
    },
};

pub(crate) async fn run_host_screen_stream<Frames, Send, Encoder>(
    frames: Frames,
    screen_id: ScreenId,
    session: Rc<SessionSend<Send>>,
    cancellation: UnsyncQueue<()>,
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

    ScreenStream::<_, Encoder, _>::new(
        CancellableFrames {
            frames,
            cancellation,
        },
        max_packet_size,
        move |packet: Encoder::Packet| {
            std::future::ready(session.send_video(VideoMessage {
                screen_id,
                payload: packet.into(),
            }))
        },
    )
    .run()
    .await
    .with_context(|| format!("Failed to stream screen {}", screen_id.0))
}

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
    use std::{future::poll_fn, pin::Pin};

    use futures_core::Stream as _;

    use crate::{app::screen_stream::CancellableFrames, infra::unsync_queue::UnsyncQueue};

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
}
