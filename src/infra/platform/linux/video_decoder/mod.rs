use std::future::Future;

use futures_util::StreamExt as _;

use crate::{
    infra::platform::dma_buf::DmaBufFrame,
    kernel::{session::ReceivedVideoFrame, video_decoder::VideoDecoder},
};

pub(crate) struct GStreamerVideoDecoder;

impl VideoDecoder for GStreamerVideoDecoder {
    type Input = ReceivedVideoFrame;
    type Frame = DmaBufFrame;

    fn run<Inputs, PresentFrame, PresentFuture>(
        mut inputs: Inputs,
        _present_frame: PresentFrame,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        async move {
            while let Some(input) = inputs.next().await {
                input?;
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::platform::video_decoder::GStreamerVideoDecoder, kernel::video_decoder::VideoDecoder,
    };

    #[test]
    fn empty_decoder_accepts_its_platform_boundary() {
        let inputs = futures_util::stream::empty();
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(GStreamerVideoDecoder::run(inputs, |_| {
                std::future::ready(Ok(()))
            }))
            .expect("Empty Linux video decoder should finish cleanly");
    }
}
