pub(super) const PACKETIZED_SEND_QUEUE_CAPACITY: usize = 1;

pub(super) struct PacketizedSendSender<Packetized> {
    sender: flume::Sender<Packetized>,
}

pub(super) struct PacketizedSendReceiver<Packetized> {
    receiver: flume::Receiver<Packetized>,
    permit_sender: flume::Sender<()>,
}

pub(super) struct PacketizePermitReceiver {
    receiver: flume::Receiver<()>,
}

impl<Packetized> PacketizedSendSender<Packetized> {
    pub(super) fn channel() -> (
        Self,
        PacketizedSendReceiver<Packetized>,
        PacketizePermitReceiver,
    ) {
        let (sender, receiver) = flume::bounded(PACKETIZED_SEND_QUEUE_CAPACITY);
        let (permit_sender, permit_receiver) = flume::bounded(1);
        permit_sender
            .send(())
            .expect("packetize permit receiver should be alive during initialization");

        (
            Self { sender },
            PacketizedSendReceiver {
                receiver,
                permit_sender,
            },
            PacketizePermitReceiver {
                receiver: permit_receiver,
            },
        )
    }

    pub(super) fn send(&self, packetized: Packetized) -> eros::Result<()> {
        self.sender
            .try_send(packetized)
            .map_err(|_| eros::error!("Packetized send queue violated its permit invariant"))
    }
}

impl<Packetized> PacketizedSendReceiver<Packetized> {
    pub(super) async fn receive(&self) -> Option<Packetized> {
        let packetized = self.receiver.recv_async().await.ok()?;
        let _ = self.permit_sender.send_async(()).await;
        Some(packetized)
    }
}

impl PacketizePermitReceiver {
    pub(super) async fn acquire(&self) -> eros::Result<()> {
        self.receiver
            .recv_async()
            .await
            .map_err(|_| eros::error!("Packetize permit channel disconnected"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permit_allows_at_most_one_queued_unit() {
        let runtime = compio::runtime::Runtime::new().expect("test runtime should start");
        runtime.block_on(async {
            let (sender, receiver, permits) = PacketizedSendSender::channel();

            permits
                .acquire()
                .await
                .expect("initial permit should exist");
            sender.send(1).expect("first item should be queued");
            assert!(sender.send(2).is_err());

            assert_eq!(receiver.receive().await, Some(1));
            permits
                .acquire()
                .await
                .expect("dequeue should return permit");
            sender
                .send(2)
                .expect("returned permit should allow next item");
        });
    }
}
