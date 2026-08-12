use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
};

use crate::app::container::root::outbound_port::ResourceUsage;

const PACKETIZER_FRAME_QUEUE_CAPACITY: usize = 32;

struct PacketizerFrameQueueState<Frame> {
    frames: VecDeque<Frame>,
    closed: bool,
}

pub(super) struct PacketizerFrameQueue<Frame> {
    state: Mutex<PacketizerFrameQueueState<Frame>>,
    frame_available: Condvar,
    usage: ResourceUsage,
}

impl<Frame> PacketizerFrameQueue<Frame> {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(PacketizerFrameQueueState {
                frames: VecDeque::with_capacity(PACKETIZER_FRAME_QUEUE_CAPACITY),
                closed: false,
            }),
            frame_available: Condvar::new(),
            usage: ResourceUsage::new(0, PACKETIZER_FRAME_QUEUE_CAPACITY),
        }
    }

    pub(super) fn usage(&self) -> ResourceUsage {
        self.usage.clone()
    }

    /// Returns false when the queue is closed.
    pub(super) fn push(&self, frame: Frame) -> bool {
        let discarded_backlog = {
            let mut state = self
                .state
                .lock()
                .expect("packetizer frame queue mutex poisoned");

            if state.closed {
                return false;
            }

            let discarded_backlog =
                (state.frames.len() == PACKETIZER_FRAME_QUEUE_CAPACITY).then(|| {
                    std::mem::replace(
                        &mut state.frames,
                        VecDeque::with_capacity(PACKETIZER_FRAME_QUEUE_CAPACITY),
                    )
                });
            state.frames.push_back(frame);
            self.usage.set_used(state.frames.len());
            discarded_backlog
        };

        self.frame_available.notify_one();

        // Drop discarded frames outside the queue lock.
        drop(discarded_backlog);

        true
    }

    /// Returns None after the queue is closed and its remaining frames are drained.
    pub(super) fn blocking_pop(&self) -> Option<Frame> {
        let mut state = self
            .frame_available
            .wait_while(
                self.state
                    .lock()
                    .expect("packetizer frame queue mutex poisoned"),
                |state| state.frames.is_empty() && !state.closed,
            )
            .expect("packetizer frame queue mutex poisoned");

        let frame = state.frames.pop_front();
        self.usage.set_used(state.frames.len());
        frame
    }

    pub(super) fn close(&self) {
        {
            let mut state = self
                .state
                .lock()
                .expect("packetizer frame queue mutex poisoned");

            if state.closed {
                return;
            }

            state.closed = true;
        }

        self.frame_available.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{PACKETIZER_FRAME_QUEUE_CAPACITY, PacketizerFrameQueue};

    fn drain(queue: &PacketizerFrameQueue<usize>) -> Vec<usize> {
        queue.close();

        std::iter::from_fn(|| queue.blocking_pop()).collect()
    }

    #[test]
    fn preserves_fifo_order_before_capacity_is_reached() {
        let queue = PacketizerFrameQueue::new();

        for frame in 0..PACKETIZER_FRAME_QUEUE_CAPACITY {
            assert!(queue.push(frame));
        }

        assert_eq!(
            drain(&queue),
            (0..PACKETIZER_FRAME_QUEUE_CAPACITY).collect::<Vec<_>>()
        );
    }

    #[test]
    fn clears_the_entire_backlog_before_pushing_the_latest_frame() {
        let queue = PacketizerFrameQueue::new();

        for frame in 0..PACKETIZER_FRAME_QUEUE_CAPACITY {
            assert!(queue.push(frame));
        }

        assert!(queue.push(PACKETIZER_FRAME_QUEUE_CAPACITY));
        assert!(queue.push(PACKETIZER_FRAME_QUEUE_CAPACITY + 1));

        assert_eq!(
            drain(&queue),
            vec![
                PACKETIZER_FRAME_QUEUE_CAPACITY,
                PACKETIZER_FRAME_QUEUE_CAPACITY + 1
            ]
        );
    }

    #[test]
    fn rejects_frames_after_close() {
        let queue = PacketizerFrameQueue::new();

        queue.close();

        assert!(!queue.push(0));
        assert!(queue.blocking_pop().is_none());
    }
}
