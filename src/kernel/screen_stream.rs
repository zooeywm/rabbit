use std::{future::poll_fn, pin::Pin};

use eros::Context as _;
use futures_core::Stream;

use crate::kernel::{frame_pipeline::FramePipeline, video_encoder::VideoEncoder};

/// Drives one screen's capture, frame processing, and encoding stages.
pub struct ScreenStream<Capture, Pipeline, Encoder> {
    capture: Capture,
    pipeline: Pipeline,
    encoder: Encoder,
}

impl<Capture, Pipeline, Encoder> ScreenStream<Capture, Pipeline, Encoder>
where
    Pipeline: FramePipeline,
    Capture: Stream<Item = eros::Result<Pipeline::Input>> + Unpin,
    Encoder: VideoEncoder<Input = Pipeline::Output>,
{
    pub fn new(capture: Capture, pipeline: Pipeline, encoder: Encoder) -> Self {
        Self {
            capture,
            pipeline,
            encoder,
        }
    }

    pub async fn process_next(&mut self) -> eros::Result<Option<Vec<Encoder::Packet>>> {
        let Some(frame) = poll_fn(|context| Pin::new(&mut self.capture).poll_next(context)).await
        else {
            return Ok(None);
        };
        let frame = frame.with_context(|| "Failed to receive the next captured frame")?;
        let frame = self
            .pipeline
            .process(frame)
            .await
            .with_context(|| "Failed to process the next captured frame")?;
        let packets = self
            .encoder
            .encode(frame)
            .await
            .with_context(|| "Failed to encode the next processed frame")?;

        Ok(Some(packets))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::{Future, ready},
        pin::Pin,
        task::{Context, Poll},
    };

    use futures_core::Stream;

    use crate::kernel::{
        frame_pipeline::FramePipeline, screen_stream::ScreenStream, video_encoder::VideoEncoder,
    };

    struct CapturedFrame(u8);
    struct ProcessedFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct EncodedPacket(u8);

    struct OneFrameCapture(Option<CapturedFrame>);

    impl Stream for OneFrameCapture {
        type Item = eros::Result<CapturedFrame>;

        fn poll_next(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.0.take().map(Ok))
        }
    }

    struct EmptyPipeline;

    impl FramePipeline for EmptyPipeline {
        type Input = CapturedFrame;
        type Output = ProcessedFrame;

        fn process(
            &mut self,
            frame: Self::Input,
        ) -> impl Future<Output = eros::Result<Self::Output>> {
            ready(Ok(ProcessedFrame(frame.0)))
        }
    }

    struct EmptyEncoder;

    impl VideoEncoder for EmptyEncoder {
        type Input = ProcessedFrame;
        type Packet = EncodedPacket;

        fn encode(
            &mut self,
            frame: Self::Input,
        ) -> impl Future<Output = eros::Result<Vec<Self::Packet>>> {
            ready(Ok(vec![EncodedPacket(frame.0), EncodedPacket(frame.0 + 1)]))
        }
    }

    #[test]
    fn processes_one_captured_frame_into_encoder_packets() {
        let mut stream = ScreenStream::new(
            OneFrameCapture(Some(CapturedFrame(11))),
            EmptyPipeline,
            EmptyEncoder,
        );
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            assert_eq!(
                stream
                    .process_next()
                    .await
                    .expect("Screen stream should process one frame"),
                Some(vec![EncodedPacket(11), EncodedPacket(12)])
            );
            assert_eq!(
                stream
                    .process_next()
                    .await
                    .expect("Closed capture should not fail"),
                None
            );
        });
    }
}
