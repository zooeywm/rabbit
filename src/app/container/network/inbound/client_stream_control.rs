use crate::{
    app::container::client_stream_pipeline::inbound::DecodeUnitSender,
    domain::stream::models::vo::StreamId,
};

const CLIENT_STREAM_CONTROL_CAPACITY: usize = 32;

pub(super) enum ClientStreamControl<Input> {
    Register {
        stream_id: StreamId,
        input_sender: DecodeUnitSender<Input>,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    Unregister {
        stream_id: StreamId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
}

pub(crate) struct ClientStreamControlSender<Input> {
    sender: flume::Sender<ClientStreamControl<Input>>,
}

pub(crate) struct ClientStreamControlReceiver<Input> {
    receiver: flume::Receiver<ClientStreamControl<Input>>,
}

impl<Input> Clone for ClientStreamControlSender<Input> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

impl<Input> ClientStreamControlSender<Input> {
    pub(crate) fn channel() -> (Self, ClientStreamControlReceiver<Input>) {
        let (sender, receiver) = flume::bounded(CLIENT_STREAM_CONTROL_CAPACITY);
        (Self { sender }, ClientStreamControlReceiver { receiver })
    }

    pub(crate) async fn register(
        &self,
        stream_id: StreamId,
        input_sender: DecodeUnitSender<Input>,
    ) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);
        self.sender
            .send_async(ClientStreamControl::Register {
                stream_id,
                input_sender,
                response_sender,
            })
            .await
            .map_err(|_| eros::error!("Network worker stopped before registering Client stream"))?;
        response_receiver
            .recv_async()
            .await
            .map_err(|_| eros::error!("Network worker stopped while registering Client stream"))?
    }

    pub(crate) async fn unregister(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);
        self.sender
            .send_async(ClientStreamControl::Unregister {
                stream_id,
                response_sender,
            })
            .await
            .map_err(|_| {
                eros::error!("Network worker stopped before unregistering Client stream")
            })?;
        response_receiver
            .recv_async()
            .await
            .map_err(|_| eros::error!("Network worker stopped while unregistering Client stream"))?
    }
}

impl<Input> ClientStreamControlReceiver<Input> {
    pub(super) async fn receive(&self) -> Option<ClientStreamControl<Input>> {
        self.receiver.recv_async().await.ok()
    }
}
