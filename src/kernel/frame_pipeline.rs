use std::future::Future;

/// Transforms one owned source frame into one frame accepted by an encoder.
pub trait FramePipeline {
    type Input;
    type Output;

    fn process(&mut self, frame: Self::Input) -> impl Future<Output = eros::Result<Self::Output>>;
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};

    use crate::kernel::frame_pipeline::FramePipeline;

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneFrame(u8);

    struct EmptyFramePipeline;

    impl FramePipeline for EmptyFramePipeline {
        type Input = NonCloneFrame;
        type Output = NonCloneFrame;

        fn process(
            &mut self,
            frame: Self::Input,
        ) -> impl Future<Output = eros::Result<Self::Output>> {
            ready(Ok(frame))
        }
    }

    #[test]
    fn pipeline_can_move_a_frame_without_cloning_it() {
        let mut pipeline = EmptyFramePipeline;
        let frame = NonCloneFrame(7);
        let runtime = compio::runtime::Runtime::new().expect("Compio runtime should start");

        let output = runtime
            .block_on(pipeline.process(frame))
            .expect("empty pipeline should return its frame");

        assert_eq!(output, NonCloneFrame(7));
    }
}
