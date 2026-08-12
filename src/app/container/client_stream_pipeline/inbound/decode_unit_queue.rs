use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

use crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit;

pub(crate) const DECODE_UNIT_QUEUE_CAPACITY: usize = 15;

struct QueuedDecodeUnit<Input> {
    generation: u64,
    input: Input,
}

struct DecodeUnitQueueState<Input> {
    units: VecDeque<QueuedDecodeUnit<Input>>,
    generation: u64,
    awaiting_recovery_point: bool,
    closed: bool,
}

struct DecodeUnitQueue<Input> {
    state: Mutex<DecodeUnitQueueState<Input>>,
    unit_available: Condvar,
}

pub(crate) enum DecodeUnitPushOutcome {
    Enqueued,
    Overflowed,
    DroppedAwaitingRecovery,
    Closed,
}

pub(crate) struct DecodeUnitSender<Input> {
    queue: Arc<DecodeUnitQueue<Input>>,
}

pub(super) struct DecodeUnitReceiver<Input> {
    queue: Arc<DecodeUnitQueue<Input>>,
}

impl<Input> Clone for DecodeUnitSender<Input> {
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
        }
    }
}

impl<Input> DecodeUnitSender<Input> {
    pub(super) fn channel() -> (Self, DecodeUnitReceiver<Input>) {
        let queue = Arc::new(DecodeUnitQueue {
            state: Mutex::new(DecodeUnitQueueState {
                units: VecDeque::with_capacity(DECODE_UNIT_QUEUE_CAPACITY),
                generation: 0,
                awaiting_recovery_point: true,
                closed: false,
            }),
            unit_available: Condvar::new(),
        });

        (
            Self {
                queue: Arc::clone(&queue),
            },
            DecodeUnitReceiver { queue },
        )
    }

    pub(crate) fn close(&self) {
        self.queue.close();
    }
}

impl<Input> DecodeUnitSender<Input>
where
    Input: VideoDecodeUnit,
{
    pub(crate) fn push(&self, input: Input) -> DecodeUnitPushOutcome {
        let is_recovery_point = input.is_recovery_point();
        let mut discarded = VecDeque::new();
        let outcome = {
            let mut state = self
                .queue
                .state
                .lock()
                .expect("decode unit queue mutex poisoned");

            if state.closed {
                return DecodeUnitPushOutcome::Closed;
            }

            if state.awaiting_recovery_point {
                if !is_recovery_point {
                    return DecodeUnitPushOutcome::DroppedAwaitingRecovery;
                }
                state.awaiting_recovery_point = false;
            }

            if state.units.len() == DECODE_UNIT_QUEUE_CAPACITY {
                state.generation = state.generation.wrapping_add(1);
                discarded = std::mem::take(&mut state.units);
                state.awaiting_recovery_point = !is_recovery_point;

                if is_recovery_point {
                    let generation = state.generation;
                    state
                        .units
                        .push_back(QueuedDecodeUnit { generation, input });
                    self.queue.unit_available.notify_one();
                }
                DecodeUnitPushOutcome::Overflowed
            } else {
                let generation = state.generation;
                state
                    .units
                    .push_back(QueuedDecodeUnit { generation, input });
                self.queue.unit_available.notify_one();
                DecodeUnitPushOutcome::Enqueued
            }
        };

        // Compressed buffers may have expensive destructors; release them outside the queue lock.
        drop(discarded);
        outcome
    }
}

impl<Input> DecodeUnitReceiver<Input> {
    pub(super) fn blocking_receive(&self) -> Option<(u64, Input)> {
        let mut state = self
            .queue
            .unit_available
            .wait_while(
                self.queue
                    .state
                    .lock()
                    .expect("decode unit queue mutex poisoned"),
                |state| state.units.is_empty() && !state.closed,
            )
            .expect("decode unit queue mutex poisoned");

        let unit = state.units.pop_front()?;
        Some((unit.generation, unit.input))
    }

    pub(super) fn generation(&self) -> u64 {
        self.queue
            .state
            .lock()
            .expect("decode unit queue mutex poisoned")
            .generation
    }
}

impl<Input> DecodeUnitQueue<Input> {
    fn close(&self) {
        let discarded = {
            let mut state = self.state.lock().expect("decode unit queue mutex poisoned");
            if state.closed {
                return;
            }
            state.closed = true;
            std::mem::take(&mut state.units)
        };
        self.unit_available.notify_all();
        drop(discarded);
    }
}

impl<Input> Drop for DecodeUnitReceiver<Input> {
    fn drop(&mut self) {
        self.queue.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestUnit {
        value: usize,
        recovery_point: bool,
    }

    impl VideoDecodeUnit for TestUnit {
        fn is_recovery_point(&self) -> bool {
            self.recovery_point
        }
    }

    #[test]
    fn starts_by_dropping_units_until_a_recovery_point_arrives() {
        let (sender, receiver) = DecodeUnitSender::channel();

        assert!(matches!(
            sender.push(TestUnit {
                value: 1,
                recovery_point: false,
            }),
            DecodeUnitPushOutcome::DroppedAwaitingRecovery
        ));
        assert!(matches!(
            sender.push(TestUnit {
                value: 2,
                recovery_point: true,
            }),
            DecodeUnitPushOutcome::Enqueued
        ));

        let (_, unit) = receiver
            .blocking_receive()
            .expect("recovery point should start the queue");
        assert_eq!(unit.value, 2);
    }

    #[test]
    fn overflow_flushes_backlog_and_waits_for_a_recovery_point() {
        let (sender, receiver) = DecodeUnitSender::channel();
        for value in 0..DECODE_UNIT_QUEUE_CAPACITY {
            assert!(matches!(
                sender.push(TestUnit {
                    value,
                    recovery_point: value == 0,
                }),
                DecodeUnitPushOutcome::Enqueued
            ));
        }

        assert!(matches!(
            sender.push(TestUnit {
                value: 100,
                recovery_point: false,
            }),
            DecodeUnitPushOutcome::Overflowed
        ));
        assert!(matches!(
            sender.push(TestUnit {
                value: 101,
                recovery_point: false,
            }),
            DecodeUnitPushOutcome::DroppedAwaitingRecovery
        ));
        assert!(matches!(
            sender.push(TestUnit {
                value: 102,
                recovery_point: true,
            }),
            DecodeUnitPushOutcome::Enqueued
        ));

        let (generation, unit) = receiver
            .blocking_receive()
            .expect("recovery unit should remain queued");
        assert_eq!(generation, 1);
        assert_eq!(unit.value, 102);
    }

    #[test]
    fn overflow_keeps_a_newest_recovery_point() {
        let (sender, receiver) = DecodeUnitSender::channel();
        for value in 0..DECODE_UNIT_QUEUE_CAPACITY {
            let _ = sender.push(TestUnit {
                value,
                recovery_point: value == 0,
            });
        }

        assert!(matches!(
            sender.push(TestUnit {
                value: 200,
                recovery_point: true,
            }),
            DecodeUnitPushOutcome::Overflowed
        ));

        let (generation, unit) = receiver
            .blocking_receive()
            .expect("latest recovery unit should remain queued");
        assert_eq!(generation, 1);
        assert_eq!(unit.value, 200);
    }
}
