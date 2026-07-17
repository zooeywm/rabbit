use std::future::Future;

/// Encodes one owned pipeline frame into zero or more owned packets.
pub trait VideoEncoder {
    type Input;
    type Packet;

    fn encode(
        &mut self,
        frame: Self::Input,
    ) -> impl Future<Output = eros::Result<Vec<Self::Packet>>>;
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};

    use crate::kernel::video_encoder::VideoEncoder;

    struct NonCloneFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct NonClonePacket(u8);

    struct EmptyVideoEncoder;

    impl VideoEncoder for EmptyVideoEncoder {
        type Input = NonCloneFrame;
        type Packet = NonClonePacket;

        fn encode(
            &mut self,
            frame: Self::Input,
        ) -> impl Future<Output = eros::Result<Vec<Self::Packet>>> {
            ready(Ok(vec![
                NonClonePacket(frame.0),
                NonClonePacket(frame.0 + 1),
            ]))
        }
    }

    #[test]
    fn encoder_can_move_a_frame_into_multiple_packets() {
        let mut encoder = EmptyVideoEncoder;
        let frame = NonCloneFrame(9);
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        let packets = runtime
            .block_on(encoder.encode(frame))
            .expect("Encoder should return packet batch");

        assert_eq!(packets, vec![NonClonePacket(9), NonClonePacket(10)]);
    }
}
