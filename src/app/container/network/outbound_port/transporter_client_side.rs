use crate::domain::stream::models::{StreamRequest, vo::StreamId};

pub(crate) trait TransporterClientSide {
    type Receiver: 'static;
    type RequestSender: 'static;
    type Received: 'static;
    type Depacketized: Send + 'static;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver>;
    fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender>;

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>>;

    async fn send_request(
        sender: &mut Self::RequestSender,
        request: StreamRequest,
    ) -> eros::Result<()>;

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(StreamId, Self::Depacketized)>;

    fn request_video_refresh(&mut self, stream_id: StreamId) -> eros::Result<()>;
}
