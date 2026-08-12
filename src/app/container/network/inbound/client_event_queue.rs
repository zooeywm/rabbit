use crate::domain::stream::models::vo::StreamId;

const NETWORK_CLIENT_EVENT_QUEUE_CAPACITY: usize = 1;

pub(crate) struct NetworkClientEvent<Input> {
    pub(crate) stream_id: StreamId,
    pub(crate) input: Input,
}

pub(super) struct NetworkClientEventSender<Input> {
    sender: flume::Sender<NetworkClientEvent<Input>>,
    permit_receiver: flume::Receiver<()>,
}

pub(crate) struct NetworkClientEventReceiver<Input> {
    receiver: flume::Receiver<NetworkClientEvent<Input>>,
    permit_sender: flume::Sender<()>,
}

impl<Input> NetworkClientEventSender<Input> {
    pub(super) fn channel() -> (Self, NetworkClientEventReceiver<Input>) {
        let (sender, receiver) = flume::bounded(NETWORK_CLIENT_EVENT_QUEUE_CAPACITY);
        let (permit_sender, permit_receiver) = flume::bounded(1);
        permit_sender
            .send(())
            .expect("network client permit receiver should be alive during initialization");

        (
            Self {
                sender,
                permit_receiver,
            },
            NetworkClientEventReceiver {
                receiver,
                permit_sender,
            },
        )
    }

    pub(super) async fn acquire_permit(&self) -> eros::Result<()> {
        self.permit_receiver
            .recv_async()
            .await
            .map_err(|_| eros::error!("Client stopped receiving network events"))
    }

    pub(super) fn send(&self, event: NetworkClientEvent<Input>) -> eros::Result<()> {
        self.sender
            .try_send(event)
            .map_err(|_| eros::error!("Network client event queue violated its permit invariant"))
    }
}

impl<Input> NetworkClientEventReceiver<Input> {
    pub(crate) async fn receive(&self) -> Option<NetworkClientEvent<Input>> {
        let event = self.receiver.recv_async().await.ok()?;
        let _ = self.permit_sender.send_async(()).await;
        Some(event)
    }
}
