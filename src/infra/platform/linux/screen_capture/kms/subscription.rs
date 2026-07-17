use std::{
    cell::RefCell,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

use futures_core::Stream;

use crate::{
    infra::platform::screen_capture::kms::types::{DmaBufFrame, KmsPlaneIssue},
    kernel::screen_capture::CapturedFrame,
};

type SharedKmsFrame = Rc<CapturedFrame<DmaBufFrame, KmsPlaneIssue>>;

#[derive(Debug, Default)]
pub(crate) struct KmsFramePublisher {
    subscribers: Vec<Weak<RefCell<KmsFrameSubscriptionState>>>,
}

#[derive(Debug)]
pub(crate) struct KmsFrameSubscription {
    state: Rc<RefCell<KmsFrameSubscriptionState>>,
}

#[derive(Debug, Default)]
struct KmsFrameSubscriptionState {
    latest: Option<SharedKmsFrame>,
    waker: Option<Waker>,
}

impl KmsFramePublisher {
    pub(crate) fn subscribe(&mut self) -> KmsFrameSubscription {
        let state = Rc::new(RefCell::new(KmsFrameSubscriptionState::default()));
        self.subscribers.push(Rc::downgrade(&state));

        KmsFrameSubscription { state }
    }

    pub(crate) fn publish(&mut self, frame: CapturedFrame<DmaBufFrame, KmsPlaneIssue>) {
        let frame = Rc::new(frame);

        self.subscribers.retain(|subscriber| {
            let Some(state) = subscriber.upgrade() else {
                return false;
            };
            let waker = {
                let mut state = state.borrow_mut();
                state.latest = Some(Rc::clone(&frame));
                state.waker.take()
            };

            if let Some(waker) = waker {
                waker.wake();
            }

            true
        });
    }
}

impl Stream for KmsFrameSubscription {
    type Item = eros::Result<SharedKmsFrame>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self.state.borrow_mut();

        if let Some(frame) = state.latest.take() {
            return Poll::Ready(Some(Ok(frame)));
        }

        match &state.waker {
            Some(waker) if waker.will_wake(context.waker()) => {}
            _ => state.waker = Some(context.waker().clone()),
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        rc::Rc,
        task::{Context, Poll, Waker},
    };

    use drm::buffer::DrmFourcc;
    use futures_core::Stream;

    use crate::{
        infra::platform::screen_capture::kms::{
            subscription::KmsFramePublisher,
            types::{DmaBufFrame, KmsPlaneIssue},
        },
        kernel::{geometry::PixelSize, screen_capture::CapturedFrame},
    };

    #[test]
    fn slow_subscribers_share_only_the_latest_frame() {
        let mut publisher = KmsFramePublisher::default();
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();

        publisher.publish(frame(1));
        publisher.publish(frame(2));

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let Poll::Ready(Some(Ok(first_frame))) = Pin::new(&mut first).poll_next(&mut context)
        else {
            panic!("first subscriber should receive the latest frame");
        };
        let Poll::Ready(Some(Ok(second_frame))) = Pin::new(&mut second).poll_next(&mut context)
        else {
            panic!("second subscriber should receive the latest frame");
        };

        assert_eq!(first_frame.buffer.size.width, 2);
        assert!(Rc::ptr_eq(&first_frame, &second_frame));
        assert!(matches!(
            Pin::new(&mut first).poll_next(&mut context),
            Poll::Pending
        ));
    }

    fn frame(width: u32) -> CapturedFrame<DmaBufFrame, KmsPlaneIssue> {
        CapturedFrame {
            buffer: DmaBufFrame {
                size: PixelSize { width, height: 1 },
                format: DrmFourcc::Xrgb8888,
                objects: Vec::new(),
                planes: Vec::new(),
                readiness_fence: None,
            },
            issues: Vec::new(),
        }
    }
}
