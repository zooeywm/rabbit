use crate::domain::stream::models::vo::StreamId;

pub(crate) trait TransporterClientSide {
    type Receiver: 'static;
    type Received: 'static;
    type Depacketized: Send + 'static;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver>;

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>>;

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(StreamId, Self::Depacketized)>;

    fn request_video_refresh(&mut self, stream_id: StreamId) -> eros::Result<()>;
}
