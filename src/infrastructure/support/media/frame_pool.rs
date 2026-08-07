use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Condvar, Mutex, PoisonError},
};

pub(crate) struct FramePool<Frame> {
    inner: Arc<FramePoolInner<Frame>>,
}

struct FramePoolInner<Frame> {
    state: Mutex<FramePoolState<Frame>>,
    frame_available: Condvar,
}

struct FramePoolState<Frame> {
    /// Every non-retiring slot participates in frame allocation.
    ///
    /// A retiring slot is still leased and will be removed when its frame
    /// is returned.
    slots: Vec<FrameSlot<Frame>>,
    /// A stable identifier assigned to the next newly created slot.
    next_slot_id: usize,

    /// The index from which the next allocation search begins.
    next_index: usize,

    wake_requested: bool,
}

struct FrameSlot<Frame> {
    /// The identifier remains stable even when other slots are removed.
    id: usize,

    /// Some(Frame): the slot is idle and can be acquired.
    /// None: the frame is currently leased.
    frame: Option<Frame>,

    /// A retiring slot is removed instead of reused when its frame returns.
    retiring: bool,
}

pub(crate) struct FrameLease<Frame> {
    inner: Arc<FrameLeaseInner<Frame>>,
}

pub(crate) struct FramePoolWaker<Frame> {
    inner: Arc<FramePoolInner<Frame>>,
}

struct FrameLeaseInner<Frame> {
    // Option allows Drop to move the frame back into the pool with take().
    frame: Option<Frame>,

    pool: Arc<FramePoolInner<Frame>>,

    // The stable slot identifier to which this frame belongs.
    slot_id: usize,
}

impl<Frame: Default> FramePool<Frame> {
    pub(crate) fn new(pool_size: usize) -> Self {
        let slots = (0..pool_size)
            .map(|id| FrameSlot {
                id,
                frame: Some(Frame::default()),
                retiring: false,
            })
            .collect();

        Self {
            inner: Arc::new(FramePoolInner {
                state: Mutex::new(FramePoolState {
                    slots,
                    next_slot_id: pool_size,
                    next_index: 0,
                    wake_requested: false,
                }),
                frame_available: Condvar::new(),
            }),
        }
    }

    pub(crate) fn set_pool_size(&mut self, pool_size: usize) {
        let (added_available_frames, removed_frames) = {
            let mut state = self.inner.state.lock().expect("frame pool mutex poisoned");

            let current_pool_size = state.slots.iter().filter(|slot| !slot.retiring).count();

            let mut added_available_frames = 0;
            let mut removed_frames = Vec::new();

            if pool_size > current_pool_size {
                let mut additional_count = pool_size - current_pool_size;

                // Reuse leased slots whose retirement has not completed yet.
                for slot in &mut state.slots {
                    if additional_count == 0 {
                        break;
                    }

                    if slot.retiring {
                        debug_assert!(slot.frame.is_none(), "a retiring slot must still be leased",);

                        slot.retiring = false;
                        additional_count -= 1;
                    }
                }

                // Create new idle slots for any remaining capacity.
                for _ in 0..additional_count {
                    let slot_id = state.next_slot_id;

                    state.next_slot_id = state
                        .next_slot_id
                        .checked_add(1)
                        .expect("frame slot id overflow");

                    state.slots.push(FrameSlot {
                        id: slot_id,
                        frame: Some(Frame::default()),
                        retiring: false,
                    });

                    added_available_frames += 1;
                }
            } else if pool_size < current_pool_size {
                let mut reduction_count = current_pool_size - pool_size;

                // Remove idle slots immediately.
                let mut index = state.slots.len();

                while index > 0 && reduction_count > 0 {
                    index -= 1;

                    let removable = {
                        let slot = &state.slots[index];

                        !slot.retiring && slot.frame.is_some()
                    };

                    if removable {
                        let slot = state.slots.remove(index);

                        removed_frames.push(slot.frame.expect("an idle slot must contain a frame"));

                        reduction_count -= 1;
                    }
                }

                // Leased slots cannot be removed immediately.
                // Mark them for removal when their frames are returned.
                if reduction_count > 0 {
                    for slot in state.slots.iter_mut().rev() {
                        if reduction_count == 0 {
                            break;
                        }

                        if !slot.retiring {
                            debug_assert!(
                                slot.frame.is_none(),
                                "all remaining active slots must be leased",
                            );

                            slot.retiring = true;
                            reduction_count -= 1;
                        }
                    }
                }

                debug_assert_eq!(
                    reduction_count, 0,
                    "the requested pool reduction must be satisfied",
                );
            }

            if pool_size != current_pool_size {
                state.next_index = 0;
            }

            (added_available_frames, removed_frames)
        }; // Frame pool state lock dropped here.

        // Removed idle frames are destroyed outside the state lock.
        drop(removed_frames);

        if added_available_frames > 0 {
            self.inner.frame_available.notify_all();
        }
    }
}

impl<Frame> FramePool<Frame> {
    pub(crate) fn blocking_acquire(&self) -> FrameLease<Frame> {
        self.blocking_acquire_inner(false)
            .expect("a non-interruptible frame acquire cannot be woken")
    }

    pub(crate) fn blocking_acquire_interruptibly(&self) -> Option<FrameLease<Frame>> {
        self.blocking_acquire_inner(true)
    }

    pub(crate) fn waker(&self) -> FramePoolWaker<Frame> {
        FramePoolWaker {
            inner: Arc::clone(&self.inner),
        }
    }

    fn blocking_acquire_inner(&self, interruptible: bool) -> Option<FrameLease<Frame>> {
        let acquired = {
            let mut state = self.inner.state.lock().expect("frame pool mutex poisoned");

            loop {
                if interruptible && state.wake_requested {
                    state.wake_requested = false;
                    break None;
                }

                let slot_count = state.slots.len();
                let mut acquired = None;

                if slot_count > 0 {
                    let mut index = if state.next_index < slot_count {
                        state.next_index
                    } else {
                        0
                    };

                    for _ in 0..slot_count {
                        let slot = &mut state.slots[index];

                        if !slot.retiring
                            && let Some(frame) = slot.frame.take()
                        {
                            acquired = Some((slot.id, frame, index));
                            break;
                        }

                        index += 1;

                        if index == slot_count {
                            index = 0;
                        }
                    }
                }

                if let Some((slot_id, frame, slot_index)) = acquired {
                    state.next_index = slot_index + 1;

                    if state.next_index == slot_count {
                        state.next_index = 0;
                    }

                    break Some((slot_id, frame));
                }

                state = self
                    .inner
                    .frame_available
                    .wait(state)
                    .expect("frame pool mutex poisoned");
            }
        };

        let (slot_id, frame) = acquired?;

        Some(FrameLease {
            inner: Arc::new(FrameLeaseInner {
                frame: Some(frame),
                pool: Arc::clone(&self.inner),
                slot_id,
            }),
        })
    }
}

impl<Frame> FramePoolWaker<Frame> {
    pub(crate) fn wake(&self) {
        self.inner
            .state
            .lock()
            .expect("frame pool mutex poisoned")
            .wake_requested = true;
        self.inner.frame_available.notify_all();
    }
}

impl<Frame> Clone for FrameLease<Frame> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<Frame> Deref for FrameLease<Frame> {
    type Target = Frame;

    fn deref(&self) -> &Self::Target {
        self.inner
            .frame
            .as_ref()
            .expect("frame lease must contain a frame")
    }
}

impl<Frame> DerefMut for FrameLease<Frame> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::get_mut(&mut self.inner)
            .expect("a shared frame lease cannot be mutably borrowed")
            .frame
            .as_mut()
            .expect("frame lease must contain a frame")
    }
}

impl<Frame> Drop for FrameLeaseInner<Frame> {
    fn drop(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };

        let mut returned_frame = Some(frame);

        let returned_to_pool = {
            let mut state = self
                .pool
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            let slot_index = state
                .slots
                .iter()
                .position(|slot| slot.id == self.slot_id)
                .expect("frame lease slot must still exist");

            if state.slots[slot_index].retiring {
                debug_assert!(
                    state.slots[slot_index].frame.is_none(),
                    "a retiring slot must not contain an idle frame",
                );

                state.slots.remove(slot_index);

                if state.slots.is_empty() {
                    state.next_index = 0;
                } else if state.next_index >= state.slots.len() {
                    state.next_index %= state.slots.len();
                }

                false
            } else {
                let slot = &mut state.slots[slot_index];

                debug_assert!(slot.frame.is_none(), "a leased slot must be empty",);

                slot.frame = returned_frame.take();

                true
            }
        }; // Frame pool state lock dropped here.

        if returned_to_pool {
            self.pool.frame_available.notify_one();
        }

        // A retired frame is destroyed here, after the state lock is released.
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc::{self, RecvTimeoutError},
        },
        thread,
        time::{Duration, Instant},
    };

    use super::{FrameLease, FramePool};

    #[derive(Default)]
    struct TestFrame {
        value: usize,
    }

    fn pool_state(pool: &FramePool<TestFrame>) -> (usize, usize, usize) {
        let state = pool.inner.state.lock().expect("frame pool mutex poisoned");

        let slot_count = state.slots.len();

        let retiring_count = state.slots.iter().filter(|slot| slot.retiring).count();

        let idle_count = state
            .slots
            .iter()
            .filter(|slot| slot.frame.is_some())
            .count();

        (slot_count, retiring_count, idle_count)
    }

    fn spawn_encoder(
        encode_duration: Duration,
    ) -> (
        mpsc::Sender<FrameLease<TestFrame>>,
        thread::JoinHandle<()>,
        Arc<AtomicUsize>,
    ) {
        let (frame_sender, frame_receiver) = mpsc::channel::<FrameLease<TestFrame>>();
        let encoded_count = Arc::new(AtomicUsize::new(0));
        let worker_encoded_count = Arc::clone(&encoded_count);

        let encoder_worker = thread::spawn(move || {
            while let Ok(frame) = frame_receiver.recv() {
                let _frame_number = frame.value;

                thread::sleep(encode_duration);
                worker_encoded_count.fetch_add(1, Ordering::SeqCst);
            }
        });

        (frame_sender, encoder_worker, encoded_count)
    }

    #[test]
    fn returned_frame_is_available_again() {
        let pool = FramePool::<TestFrame>::new(1);

        let mut frame = pool.blocking_acquire();
        frame.value = 42;

        assert_eq!(pool_state(&pool), (1, 0, 0));

        drop(frame);

        assert_eq!(pool_state(&pool), (1, 0, 1));

        let frame = pool.blocking_acquire();

        assert_eq!(frame.value, 42);
        assert_eq!(pool_state(&pool), (1, 0, 0));
    }

    #[test]
    fn frame_returns_only_after_last_lease_is_dropped() {
        let pool = Arc::new(FramePool::<TestFrame>::new(1));

        let frame = pool.blocking_acquire();
        let cloned_frame = frame.clone();

        let worker_pool = Arc::clone(&pool);

        let (started_sender, started_receiver) = mpsc::channel();

        let (acquired_sender, acquired_receiver) = mpsc::channel();

        let worker = thread::spawn(move || {
            started_sender
                .send(())
                .expect("failed to report worker start");

            let _frame = worker_pool.blocking_acquire();

            acquired_sender
                .send(())
                .expect("failed to report frame acquisition");
        });

        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker did not start");

        drop(frame);

        assert!(matches!(
            acquired_receiver.recv_timeout(Duration::from_millis(100),),
            Err(RecvTimeoutError::Timeout),
        ));

        drop(cloned_frame);

        acquired_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("frame was not returned after the last lease was dropped");

        worker.join().expect("worker thread panicked");
    }

    #[test]
    fn shrinking_removes_idle_slots_immediately() {
        let mut pool = FramePool::<TestFrame>::new(3);

        let frame = pool.blocking_acquire();

        assert_eq!(pool_state(&pool), (3, 0, 2));

        pool.set_pool_size(1);

        assert_eq!(pool_state(&pool), (1, 0, 0));

        drop(frame);

        assert_eq!(pool_state(&pool), (1, 0, 1));
    }

    #[test]
    fn shrinking_marks_leased_slots_as_retiring() {
        let mut pool = FramePool::<TestFrame>::new(2);

        let first_frame = pool.blocking_acquire();
        let second_frame = pool.blocking_acquire();

        assert_eq!(pool_state(&pool), (2, 0, 0));

        pool.set_pool_size(1);

        assert_eq!(pool_state(&pool), (2, 1, 0));

        drop(first_frame);
        drop(second_frame);

        assert_eq!(pool_state(&pool), (1, 0, 1));
    }

    #[test]
    fn growing_cancels_pending_retirement() {
        let mut pool = FramePool::<TestFrame>::new(2);

        let first_frame = pool.blocking_acquire();
        let second_frame = pool.blocking_acquire();

        pool.set_pool_size(1);

        assert_eq!(pool_state(&pool), (2, 1, 0));

        pool.set_pool_size(2);

        assert_eq!(pool_state(&pool), (2, 0, 0));

        drop(first_frame);
        drop(second_frame);

        assert_eq!(pool_state(&pool), (2, 0, 2));
    }

    fn simulate_capture_and_encoding(
        pool: &FramePool<TestFrame>,
        stream_senders: &[mpsc::Sender<FrameLease<TestFrame>>],
        encoded_counts: &[Arc<AtomicUsize>],
        capture_interval: Duration,
        simulation_duration: Duration,
        frame_number: &mut usize,
    ) {
        let counts_before_interval = encoded_counts
            .iter()
            .map(|count| count.load(Ordering::SeqCst))
            .collect::<Vec<_>>();
        let interval_deadline = Instant::now() + simulation_duration;
        let mut captured_in_interval = 0;

        while Instant::now() < interval_deadline {
            let mut frame = pool.blocking_acquire();

            *frame_number += 1;
            frame.value = *frame_number;

            for stream_sender in stream_senders {
                stream_sender
                    .send(frame.clone())
                    .expect("encoder thread stopped before capture completed");
            }

            drop(frame);

            thread::sleep(capture_interval);

            captured_in_interval += 1;
        }

        assert!(captured_in_interval > 0);

        for (encoded_count, count_before_interval) in
            encoded_counts.iter().zip(counts_before_interval)
        {
            assert!(encoded_count.load(Ordering::SeqCst) > count_before_interval);
        }
    }

    fn run_single_capture_pool_resize_test(capture_interval: Duration, encode_duration: Duration) {
        const STREAM_CHANGE_INTERVAL: Duration = Duration::from_millis(50);
        const NEW_STREAM_COUNT: usize = 5;
        const TOTAL_STREAM_COUNT: usize = NEW_STREAM_COUNT + 1;

        let mut pool = FramePool::<TestFrame>::new(1);
        let mut stream_senders = Vec::with_capacity(TOTAL_STREAM_COUNT);
        let mut encoder_workers = Vec::with_capacity(TOTAL_STREAM_COUNT);
        let mut encoded_counts = Vec::with_capacity(TOTAL_STREAM_COUNT);
        let mut encoder_start_frame_numbers = Vec::with_capacity(TOTAL_STREAM_COUNT);
        let mut frame_number = 0;

        for stream_count in 1..=TOTAL_STREAM_COUNT {
            if stream_count > 1 {
                pool.set_pool_size(stream_count);
            }

            let (slot_count, retiring_count, _) = pool_state(&pool);

            assert_eq!(slot_count, stream_count);
            assert_eq!(retiring_count, 0);

            let (frame_sender, encoder_worker, encoded_count) = spawn_encoder(encode_duration);

            stream_senders.push(frame_sender);
            encoder_workers.push(encoder_worker);
            encoded_counts.push(encoded_count);
            encoder_start_frame_numbers.push(frame_number);

            simulate_capture_and_encoding(
                &pool,
                &stream_senders,
                &encoded_counts,
                capture_interval,
                STREAM_CHANGE_INTERVAL,
                &mut frame_number,
            );
        }

        for remaining_stream_count in (0..TOTAL_STREAM_COUNT).rev() {
            drop(
                stream_senders
                    .pop()
                    .expect("the stream sender should exist"),
            );

            encoder_workers
                .pop()
                .expect("the encoder worker should exist")
                .join()
                .expect("encoder thread panicked");

            let encoded_count = encoded_counts
                .pop()
                .expect("the encoded count should exist");
            let start_frame_number = encoder_start_frame_numbers
                .pop()
                .expect("the encoder start frame number should exist");

            assert_eq!(
                encoded_count.load(Ordering::SeqCst),
                frame_number - start_frame_number,
            );

            pool.set_pool_size(remaining_stream_count);

            let (slot_count, retiring_count, _) = pool_state(&pool);

            assert_eq!(slot_count - retiring_count, remaining_stream_count);

            if remaining_stream_count > 0 {
                simulate_capture_and_encoding(
                    &pool,
                    &stream_senders,
                    &encoded_counts,
                    capture_interval,
                    STREAM_CHANGE_INTERVAL,
                    &mut frame_number,
                );
            }
        }

        assert_eq!(pool_state(&pool), (0, 0, 0));
        assert!(stream_senders.is_empty());
        assert!(encoder_workers.is_empty());
        assert!(encoded_counts.is_empty());
        assert!(encoder_start_frame_numbers.is_empty());

        pool.set_pool_size(1);

        let (frame_sender, encoder_worker, encoded_count) = spawn_encoder(encode_duration);
        let recreated_stream_start_frame_number = frame_number;

        simulate_capture_and_encoding(
            &pool,
            std::slice::from_ref(&frame_sender),
            std::slice::from_ref(&encoded_count),
            capture_interval,
            STREAM_CHANGE_INTERVAL,
            &mut frame_number,
        );

        drop(frame_sender);
        encoder_worker.join().expect("encoder thread panicked");

        assert_eq!(
            encoded_count.load(Ordering::SeqCst),
            frame_number - recreated_stream_start_frame_number,
        );

        pool.set_pool_size(0);

        assert_eq!(pool_state(&pool), (0, 0, 0));
    }

    #[test]
    fn pool_grows_and_shrinks_while_capture_is_faster_than_encoding() {
        run_single_capture_pool_resize_test(Duration::from_millis(1), Duration::from_millis(3));
    }

    #[test]
    fn pool_grows_and_shrinks_while_encoding_is_faster_than_capture() {
        run_single_capture_pool_resize_test(Duration::from_millis(3), Duration::from_millis(1));
    }

    #[test]
    fn pool_grows_and_shrinks_while_capture_is_faster_than_encoding_2() {
        run_single_capture_pool_resize_test(Duration::from_millis(1), Duration::from_millis(10));
    }

    #[test]
    fn pool_grows_and_shrinks_while_encoding_is_faster_than_capture_2() {
        run_single_capture_pool_resize_test(Duration::from_millis(10), Duration::from_millis(1));
    }

    #[test]
    fn pool_grows_and_shrinks_while_encoding_is_equal_to_capture() {
        run_single_capture_pool_resize_test(Duration::from_millis(1), Duration::from_millis(1));
    }

    #[test]
    fn pool_grows_and_shrinks_while_encoding_is_equal_to_capture_2() {
        run_single_capture_pool_resize_test(Duration::from_millis(10), Duration::from_millis(10));
    }

    #[test]
    fn shrinking_to_zero_removes_frames_after_return() {
        let mut pool = FramePool::<TestFrame>::new(2);

        let first_frame = pool.blocking_acquire();
        let second_frame = pool.blocking_acquire();

        pool.set_pool_size(0);

        assert_eq!(pool_state(&pool), (2, 2, 0));

        drop(first_frame);

        assert_eq!(pool_state(&pool), (1, 1, 0));

        drop(second_frame);

        assert_eq!(pool_state(&pool), (0, 0, 0));
    }
}
