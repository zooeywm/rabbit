use std::sync::{Condvar, Mutex, PoisonError};

struct LatestFrameSlotState<Frame> {
    frame: Option<Frame>,
    closed: bool,
}

pub(crate) struct LatestFrameSlot<Frame> {
    state: Mutex<LatestFrameSlotState<Frame>>,
    frame_available: Condvar,
}

impl<Frame> LatestFrameSlot<Frame> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(LatestFrameSlotState {
                frame: None,
                closed: false,
            }),
            frame_available: Condvar::new(),
        }
    }

    /// Return false when slot closed.
    pub(crate) fn replace(&self, frame: Frame) -> bool {
        let _replaced_frame = {
            let mut state = self.state.lock().expect("latest frame slot mutex poisoned");

            if state.closed {
                return false;
            }

            state.frame.replace(frame)
        };

        self.frame_available.notify_one();

        // _replaced_frame drop here

        true
    }

    /// Return None when slot closed.
    pub(crate) fn blocking_take(&self) -> Option<Frame> {
        let mut state = self
            .frame_available
            .wait_while(
                self.state.lock().expect("latest frame slot mutex poisoned"),
                |state| state.frame.is_none() && !state.closed,
            )
            .expect("latest frame slot mutex poisoned");

        state.frame.take()
    }

    pub(crate) fn close(&self) {
        let _pending_frame = {
            let mut state = self.state.lock().expect("latest frame slot mutex poisoned");

            if state.closed {
                return;
            }

            state.closed = true;
            state.frame.take()
        };

        // _pending_frame drop here

        self.frame_available.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc::{self, RecvTimeoutError},
        },
        thread,
        time::Duration,
    };

    use super::LatestFrameSlot;

    #[derive(Debug, PartialEq, Eq)]
    struct TestFrame {
        value: usize,
    }

    struct DropFrame {
        drop_count: Arc<AtomicUsize>,
    }

    impl Drop for DropFrame {
        fn drop(&mut self) {
            self.drop_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn replaced_frame_can_be_taken() {
        let slot = LatestFrameSlot::new();

        assert!(slot.replace(TestFrame { value: 42 }));

        let frame = slot.blocking_take().expect("the frame should be available");

        assert_eq!(frame, TestFrame { value: 42 });
    }

    #[test]
    fn replacing_discards_the_previous_pending_frame() {
        let slot = LatestFrameSlot::new();

        assert!(slot.replace(TestFrame { value: 1 }));
        assert!(slot.replace(TestFrame { value: 2 }));

        let frame = slot
            .blocking_take()
            .expect("the latest frame should be available");

        assert_eq!(frame, TestFrame { value: 2 });
    }

    #[test]
    fn blocking_take_waits_until_a_frame_is_replaced() {
        let slot = Arc::new(LatestFrameSlot::new());
        let worker_slot = Arc::clone(&slot);

        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);

        let (sender, receiver) = mpsc::channel();

        let worker = thread::spawn(move || {
            worker_barrier.wait();

            let frame = worker_slot.blocking_take();

            sender
                .send(frame)
                .expect("failed to report the blocking_take result");
        });

        barrier.wait();

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Timeout),
        ));

        assert!(slot.replace(TestFrame { value: 42 }));

        let frame = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking_take was not awakened")
            .expect("the slot should return a frame");

        assert_eq!(frame, TestFrame { value: 42 });

        worker.join().expect("worker thread panicked");
    }

    #[test]
    fn close_wakes_a_blocked_take() {
        let slot = Arc::new(LatestFrameSlot::<TestFrame>::new());
        let worker_slot = Arc::clone(&slot);

        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);

        let (sender, receiver) = mpsc::channel();

        let worker = thread::spawn(move || {
            worker_barrier.wait();

            let frame = worker_slot.blocking_take();

            sender
                .send(frame)
                .expect("failed to report the blocking_take result");
        });

        barrier.wait();

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Timeout),
        ));

        slot.close();

        let frame = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking_take was not awakened after close");

        assert!(frame.is_none());

        worker.join().expect("worker thread panicked");
    }

    #[test]
    fn close_discards_the_pending_frame() {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let slot = LatestFrameSlot::new();

        assert!(slot.replace(DropFrame {
            drop_count: Arc::clone(&drop_count),
        }));

        assert_eq!(drop_count.load(Ordering::SeqCst), 0);

        slot.close();

        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        assert!(slot.blocking_take().is_none());
    }

    #[test]
    fn replace_rejects_and_drops_a_frame_after_close() {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let slot = LatestFrameSlot::new();

        slot.close();

        assert!(!slot.replace(DropFrame {
            drop_count: Arc::clone(&drop_count),
        }));

        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn close_is_idempotent() {
        let slot = LatestFrameSlot::<TestFrame>::new();

        slot.close();
        slot.close();

        assert!(slot.blocking_take().is_none());
        assert!(!slot.replace(TestFrame { value: 42 }));
    }
}
