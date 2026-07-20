use std::future::Future;

/// Runs one long-lived decoder over a stream of encoded video input.
pub trait VideoDecoder {
    type Input;
    type Frame;

    fn run<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>;
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, future::Future};

    use futures_util::StreamExt as _;

    use crate::kernel::video_decoder::VideoDecoder;

    struct NonCloneEncodedFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneDecodedFrame(u8);

    struct EmptyVideoDecoder;

    impl VideoDecoder for EmptyVideoDecoder {
        type Input = NonCloneEncodedFrame;
        type Frame = NonCloneDecodedFrame;

        fn run<Inputs, PresentFrame, PresentFuture>(
            mut inputs: Inputs,
            mut present_frame: PresentFrame,
        ) -> impl Future<Output = eros::Result<()>>
        where
            Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
            PresentFrame: FnMut(Self::Frame) -> PresentFuture,
            PresentFuture: Future<Output = eros::Result<()>>,
        {
            async move {
                while let Some(input) = inputs.next().await {
                    let input = input.expect("Decoder input should contain an encoded frame");
                    present_frame(NonCloneDecodedFrame(input.0)).await?;
                }

                Ok(())
            }
        }
    }

    #[test]
    fn decoder_moves_encoded_input_into_its_decoded_frame_sink() {
        let inputs = futures_util::stream::iter([Ok(NonCloneEncodedFrame(9))]);
        let frames = RefCell::new(Vec::new());
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(EmptyVideoDecoder::run(inputs, |frame| {
                frames.borrow_mut().push(frame);
                std::future::ready(Ok(()))
            }))
            .expect("Decoder should drive its complete input stream");

        assert_eq!(frames.into_inner(), vec![NonCloneDecodedFrame(9)]);
    }
}
