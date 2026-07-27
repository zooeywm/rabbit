use std::rc::Rc;

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_manager::ScreenId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FramePipelineParameters {
    pub frame_size: PixelSize,
}

/// How frames move from capture to the consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameDelivery {
    /// Low-latency streaming: keep only the newest frame at each stage.
    Latest,
    /// Recording / archival: retain every frame in order until capacity is full,
    /// then apply backpressure instead of dropping.
    Reliable {
        /// Max frames buffered between GPU output and the consumer.
        capacity: usize,
    },
}

impl FrameDelivery {
    /// Default recording buffer (~2s at 144 Hz).
    pub const fn recording() -> Self {
        Self::Reliable { capacity: 288 }
    }
}

pub trait FramePipelineManager {
    type Frame;
    type Subscription: futures_core::Stream<Item = eros::Result<Rc<Self::Frame>>>;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: FramePipelineParameters,
        frame_rate: FrameRate,
        delivery: FrameDelivery,
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
        frame_pipeline::{FrameDelivery, FramePipelineManager, FramePipelineParameters},
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
            _delivery: FrameDelivery,
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
                FrameDelivery::Latest,
            )
            .expect("Frame pipeline subscription should be created");
    }
}
