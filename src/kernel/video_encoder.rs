use std::future::Future;

/// Encodes one owned pipeline frame into one owned encoded frame.
pub trait VideoEncoder {
    type Input;
    type Output;

    fn encode(&mut self, frame: Self::Input) -> impl Future<Output = eros::Result<Self::Output>>;
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};

    use crate::kernel::video_encoder::VideoEncoder;

    struct NonCloneFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneEncodedFrame(u8);

    struct EmptyVideoEncoder;

    impl VideoEncoder for EmptyVideoEncoder {
        type Input = NonCloneFrame;
        type Output = NonCloneEncodedFrame;

        fn encode(
            &mut self,
            frame: Self::Input,
        ) -> impl Future<Output = eros::Result<Self::Output>> {
            ready(Ok(NonCloneEncodedFrame(frame.0)))
        }
    }

    #[test]
    fn encoder_can_move_a_frame_without_cloning_it() {
        let mut encoder = EmptyVideoEncoder;
        let frame = NonCloneFrame(9);
        let runtime = compio::runtime::Runtime::new().expect("Compio runtime should start");

        let output = runtime
            .block_on(encoder.encode(frame))
            .expect("empty encoder should return an encoded frame");

        assert_eq!(output, NonCloneEncodedFrame(9));
    }
}
