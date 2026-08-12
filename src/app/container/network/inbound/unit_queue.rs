use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    app::container::{ResourceUsage, stream_pipeline::outbound_port::EncodedVideoUnit},
    domain::stream::models::vo::StreamId,
};

pub(super) const ENCODED_UNIT_QUEUE_CAPACITY: usize = 32;

pub(super) struct QueuedEncodedUnit<Buffer> {
    pub(super) stream_id: StreamId,
    pub(super) unit: EncodedVideoUnit<Buffer>,
}

struct EncodedUnitQueue<Buffer> {
    sender: flume::Sender<QueuedEncodedUnit<Buffer>>,
    drain_receiver: flume::Receiver<QueuedEncodedUnit<Buffer>>,
    producer_lock: Mutex<()>,
    receiver_alive: Arc<AtomicBool>,
    usage: ResourceUsage,
}

pub(crate) struct EncodedUnitSender<Buffer> {
    queue: Arc<EncodedUnitQueue<Buffer>>,
}

pub(super) struct EncodedUnitReceiver<Buffer> {
    receiver: flume::Receiver<QueuedEncodedUnit<Buffer>>,
    receiver_alive: Arc<AtomicBool>,
    usage: ResourceUsage,
}

impl<Buffer> Clone for EncodedUnitSender<Buffer> {
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
        }
    }
}

impl<Buffer> EncodedUnitSender<Buffer> {
    pub(crate) fn send(
        &self,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Buffer>,
    ) -> eros::Result<()> {
        if !self.queue.receiver_alive.load(Ordering::Acquire) {
            eros::bail!("Network worker stopped before receiving encoded unit");
        }

        let discarded_backlog = {
            let _producer_guard = self
                .queue
                .producer_lock
                .lock()
                .expect("network queue producer mutex should not be poisoned");
            let item = QueuedEncodedUnit { stream_id, unit };
            let mut discarded_backlog = Vec::new();

            match self.queue.sender.try_send(item) {
                Ok(()) => {}
                Err(flume::TrySendError::Full(item)) => {
                    while let Ok(discarded) = self.queue.drain_receiver.try_recv() {
                        discarded_backlog.push(discarded);
                    }

                    if !self.queue.receiver_alive.load(Ordering::Acquire) {
                        eros::bail!("Network worker stopped before receiving encoded unit");
                    }

                    self.queue.sender.try_send(item).map_err(|_| {
                        eros::error!("Network worker stopped while receiving encoded unit")
                    })?;
                }
                Err(flume::TrySendError::Disconnected(_)) => {
                    eros::bail!("Network worker stopped before receiving encoded unit");
                }
            }

            self.queue.usage.set_used(self.queue.sender.len());
            discarded_backlog
        };

        // Encoded buffers may have expensive destructors; keep them outside the producer lock.
        drop(discarded_backlog);
        Ok(())
    }
}

impl<Buffer> EncodedUnitReceiver<Buffer> {
    pub(super) fn channel() -> (EncodedUnitSender<Buffer>, Self) {
        let (sender, receiver) = flume::bounded(ENCODED_UNIT_QUEUE_CAPACITY);
        let receiver_alive = Arc::new(AtomicBool::new(true));
        let usage = ResourceUsage::new(0, ENCODED_UNIT_QUEUE_CAPACITY);

        (
            EncodedUnitSender {
                queue: Arc::new(EncodedUnitQueue {
                    sender,
                    drain_receiver: receiver.clone(),
                    producer_lock: Mutex::new(()),
                    receiver_alive: Arc::clone(&receiver_alive),
                    usage: usage.clone(),
                }),
            },
            Self {
                receiver,
                receiver_alive,
                usage,
            },
        )
    }

    pub(super) fn usage(&self) -> ResourceUsage {
        self.usage.clone()
    }

    pub(super) async fn receive(&self) -> Option<QueuedEncodedUnit<Buffer>> {
        let unit = self.receiver.recv_async().await.ok();
        self.usage.set_used(self.receiver.len());
        unit
    }
}

impl<Buffer> Drop for EncodedUnitReceiver<Buffer> {
    fn drop(&mut self) {
        self.receiver_alive.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::container::stream_pipeline::outbound_port::{EncodedVideoUnit, UnitNumber},
        domain::stream::models::vo::{CaptureSourceId, FrameId},
    };

    fn unit(sequence: u64) -> EncodedVideoUnit<u64> {
        EncodedVideoUnit::new(
            FrameId::new(CaptureSourceId::new(0), sequence),
            UnitNumber::new(sequence),
            false,
            sequence,
        )
    }

    #[test]
    fn full_queue_discards_the_entire_backlog_and_keeps_the_latest_unit() {
        let (sender, receiver) = EncodedUnitReceiver::channel();

        for sequence in 0..ENCODED_UNIT_QUEUE_CAPACITY as u64 {
            sender
                .send(StreamId::new(0), unit(sequence))
                .expect("queue should accept units up to its capacity");
        }
        sender
            .send(StreamId::new(1), unit(ENCODED_UNIT_QUEUE_CAPACITY as u64))
            .expect("queue should accept the latest unit after clearing its backlog");

        let item = receiver
            .receiver
            .try_recv()
            .expect("latest unit should remain queued");
        assert_eq!(item.stream_id.value(), 1);
        assert_eq!(item.unit.data, ENCODED_UNIT_QUEUE_CAPACITY as u64);
        assert!(receiver.receiver.try_recv().is_err());
    }
}
