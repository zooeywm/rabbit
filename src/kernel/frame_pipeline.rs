use std::{future::Future, rc::Rc};

use crate::kernel::{geometry::PixelSize, screen_manager::ScreenId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FramePipelineParameters {
    pub frame_size: PixelSize,
}

pub trait FramePipelineManager {
    type Frame;
    type Subscription: futures_core::Stream<Item = eros::Result<Rc<Self::Frame>>>;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: FramePipelineParameters,
    ) -> eros::Result<Self::Subscription>;
}

/// Transforms one owned source frame into one frame accepted by an encoder.
pub trait FramePipeline {
    type Input;
    type Output;

    fn process(&mut self, frame: Self::Input) -> impl Future<Output = eros::Result<Self::Output>>;
}

#[cfg(test)]
mod tests {
    use std::{
        future::{Future, ready},
        pin::Pin,
        rc::Rc,
        task::{Context, Poll},
    };

    use futures_core::Stream;

    use crate::kernel::{
        frame_pipeline::{FramePipeline, FramePipelineManager, FramePipelineParameters},
        geometry::PixelSize,
        screen_manager::ScreenId,
    };

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneFrame(u8);

    struct EmptyFramePipeline;

    struct EmptyFramePipelineManager;

    struct EmptyFramePipelineSubscription;

    impl Stream for EmptyFramePipelineSubscription {
        type Item = eros::Result<Rc<NonCloneFrame>>;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    impl FramePipelineManager for EmptyFramePipelineManager {
        type Frame = NonCloneFrame;
        type Subscription = EmptyFramePipelineSubscription;

        fn subscribe(
            &mut self,
            _screen_id: &ScreenId,
            _parameters: FramePipelineParameters,
        ) -> eros::Result<Self::Subscription> {
            Ok(EmptyFramePipelineSubscription)
        }
    }

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

    #[test]
    fn manager_subscribes_by_screen_and_processing_parameters() {
        let mut manager = EmptyFramePipelineManager;
        let parameters = FramePipelineParameters {
            frame_size: PixelSize {
                width: 1920,
                height: 1080,
            },
        };

        manager
            .subscribe(&ScreenId(3), parameters)
            .expect("Frame pipeline subscription should be created");
    }
}
