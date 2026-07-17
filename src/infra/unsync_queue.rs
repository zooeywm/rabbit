use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use thin_cell::unsync::ThinCell;

#[derive(Debug)]
pub(crate) struct UnsyncQueue<T> {
    inner: ThinCell<Inner<T>>,
}

#[derive(Debug)]
struct Inner<T> {
    items: VecDeque<T>,
    receiver_waker: Option<Waker>,
}

impl<T> Clone for UnsyncQueue<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Default for UnsyncQueue<T> {
    fn default() -> Self {
        Self {
            inner: ThinCell::new(Inner {
                items: VecDeque::new(),
                receiver_waker: None,
            }),
        }
    }
}

impl<T> UnsyncQueue<T> {
    pub(crate) fn push(&self, item: T) {
        let receiver_waker = {
            let mut inner = self.inner.borrow();

            inner.items.push_back(item);
            inner.receiver_waker.take()
        };

        if let Some(receiver_waker) = receiver_waker {
            receiver_waker.wake();
        }
    }

    pub(crate) fn pop(&self) -> UnsyncQueuePop<'_, T> {
        UnsyncQueuePop { queue: self }
    }

    pub(crate) fn push_latest_by(&self, item: T, mut should_replace: impl FnMut(&T) -> bool) {
        let receiver_waker = {
            let mut inner = self.inner.borrow();

            match inner
                .items
                .iter_mut()
                .rev()
                .find(|queued| should_replace(queued))
            {
                Some(queued) => {
                    *queued = item;
                    None
                }
                None => {
                    inner.items.push_back(item);
                    inner.receiver_waker.take()
                }
            }
        };

        if let Some(receiver_waker) = receiver_waker {
            receiver_waker.wake();
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnsyncQueuePop<'queue, T> {
    queue: &'queue UnsyncQueue<T>,
}

impl<T> Future for UnsyncQueuePop<'_, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.queue.inner.borrow();

        if let Some(item) = inner.items.pop_front() {
            return Poll::Ready(item);
        }

        match &inner.receiver_waker {
            Some(receiver_waker) if receiver_waker.will_wake(context.waker()) => {}
            _ => inner.receiver_waker = Some(context.waker().clone()),
        }

        Poll::Pending
    }
}
