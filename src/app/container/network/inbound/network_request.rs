use crate::domain::stream::models::StreamRequest;

const NETWORK_REQUEST_CAPACITY: usize = 32;

pub(crate) struct NetworkRequestSender {
    sender: flume::Sender<StreamRequest>,
}

pub(crate) struct NetworkRequestReceiver {
    receiver: flume::Receiver<StreamRequest>,
}

impl Clone for NetworkRequestSender {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

impl NetworkRequestSender {
    pub(crate) fn channel() -> (Self, NetworkRequestReceiver) {
        let (sender, receiver) = flume::bounded(NETWORK_REQUEST_CAPACITY);
        (Self { sender }, NetworkRequestReceiver { receiver })
    }

    pub(crate) async fn send(&self, request: StreamRequest) -> eros::Result<()> {
        self.sender
            .send_async(request)
            .await
            .map_err(|_| eros::error!("Network worker stopped before receiving request"))
    }
}

impl NetworkRequestReceiver {
    pub(super) async fn receive(&self) -> Option<StreamRequest> {
        self.receiver.recv_async().await.ok()
    }
}
