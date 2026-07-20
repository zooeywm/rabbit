use std::{future::Future, rc::Rc};

/// Runs one long-lived encoder over a stream of processed video frames.
pub trait VideoEncoder {
    type Input;
    type Packet;

    fn run<Frames, SendPacket, SendFuture>(
        frames: Frames,
        max_packet_size: usize,
        send_packet: SendPacket,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
        SendPacket: FnMut(Self::Packet) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>;
}

#[cfg(test)]
mod tests {
    use std::{future::Future, rc::Rc};

    use futures_util::StreamExt as _;

    use crate::kernel::video_encoder::VideoEncoder;

    struct NonCloneFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct NonClonePacket(u8);

    struct EmptyVideoEncoder;

    impl VideoEncoder for EmptyVideoEncoder {
        type Input = NonCloneFrame;
        type Packet = NonClonePacket;

        fn run<Frames, SendPacket, SendFuture>(
            mut frames: Frames,
            _max_packet_size: usize,
            mut send_packet: SendPacket,
        ) -> impl Future<Output = eros::Result<()>>
        where
            Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
            SendPacket: FnMut(Self::Packet) -> SendFuture,
            SendFuture: Future<Output = eros::Result<()>>,
        {
            async move {
                while let Some(frame) = frames.next().await {
                    let frame = frame.expect("Encoder input should contain a frame");
                    send_packet(NonClonePacket(frame.0)).await?;
                }

                Ok(())
            }
        }
    }

    #[test]
    fn encoder_drives_a_frame_stream_into_a_packet_sink() {
        let frames = futures_util::stream::iter([Ok(Rc::new(NonCloneFrame(9)))]);
        let packets = std::cell::RefCell::new(Vec::new());
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(EmptyVideoEncoder::run(frames, 1_200, |packet| {
                packets.borrow_mut().push(packet);
                std::future::ready(Ok(()))
            }))
            .expect("Encoder should drive its complete input stream");

        assert_eq!(packets.into_inner(), vec![NonClonePacket(9)]);
    }
}
