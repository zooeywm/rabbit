use std::rc::Rc;

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_manager::ScreenId,
};

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
        frame_rate: FrameRate,
    ) -> eros::Result<Self::Subscription>;
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        rc::Rc,
        task::{Context, Poll},
    };

    use futures_core::Stream;

    use crate::kernel::{
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        geometry::{FrameRate, PixelSize},
        screen_manager::ScreenId,
    };

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneFrame(u8);

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
            _frame_rate: FrameRate,
        ) -> eros::Result<Self::Subscription> {
            Ok(EmptyFramePipelineSubscription)
        }
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
            .subscribe(
                &ScreenId(3),
                parameters,
                FrameRate::new(60, 1).expect("Test frame rate should be valid"),
            )
            .expect("Frame pipeline subscription should be created");
    }
}
