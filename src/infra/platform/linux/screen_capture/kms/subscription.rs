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
    closed: bool,
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

    pub(crate) fn has_subscribers(&mut self) -> bool {
        self.subscribers
            .retain(|subscriber| subscriber.strong_count() > 0);

        !self.subscribers.is_empty()
    }

    pub(crate) fn close(&mut self) {
        for subscriber in self.subscribers.drain(..) {
            let Some(state) = subscriber.upgrade() else {
                continue;
            };
            let waker = {
                let mut state = state.borrow_mut();
                state.closed = true;
                state.waker.take()
            };

            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl Stream for KmsFrameSubscription {
    type Item = eros::Result<SharedKmsFrame>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self.state.borrow_mut();

        if let Some(frame) = state.latest.take() {
            return Poll::Ready(Some(Ok(frame)));
        }

        if state.closed {
            return Poll::Ready(None);
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
            subscription::{KmsFramePublisher, KmsFrameSubscription, SharedKmsFrame},
            types::{DmaBufFrame, KmsPlaneIssue},
        },
        kernel::{geometry::PixelSize, screen_capture::CapturedFrame},
    };

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn slow_subscribers_share_only_the_latest_frame() {
        let mut publisher = KmsFramePublisher::default();
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();

        publisher.publish(frame(1));
        publisher.publish(frame(2));

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let first_frame = ready_frame(&mut first, &mut context);
        let second_frame = ready_frame(&mut second, &mut context);

        assert_eq!(first_frame.buffer.size.width, 2);
        assert!(Rc::ptr_eq(&first_frame, &second_frame));
        assert!(matches!(
            Pin::new(&mut first).poll_next(&mut context),
            Poll::Pending
        ));
    }

    #[test]
    #[ignore = "run through scripts/test-kms"]
    fn publisher_detects_dropped_subscribers_and_closes_live_ones() {
        let mut publisher = KmsFramePublisher::default();
        let dropped = publisher.subscribe();
        let mut live = publisher.subscribe();

        drop(dropped);
        assert!(publisher.has_subscribers());
        publisher.close();

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        assert!(matches!(
            Pin::new(&mut live).poll_next(&mut context),
            Poll::Ready(None)
        ));
        assert!(!publisher.has_subscribers());
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

    fn ready_frame(
        subscription: &mut KmsFrameSubscription,
        context: &mut Context<'_>,
    ) -> SharedKmsFrame {
        match Pin::new(subscription).poll_next(context) {
            Poll::Ready(frame) => frame
                .expect("KMS frame subscription should remain open")
                .expect("KMS frame subscription should publish a valid frame"),
            Poll::Pending => panic!("KMS frame subscription should have a published frame"),
        }
    }
}
