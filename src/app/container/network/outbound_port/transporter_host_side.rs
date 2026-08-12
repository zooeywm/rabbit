use crate::{
    app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit,
    domain::stream::models::vo::StreamId,
};

use super::SentBytes;

pub(crate) trait TransporterHostSide {
    type EncodedBuffer;
    type Packetized;
    type Sender: 'static;

    fn take_sender(&mut self) -> eros::Result<Self::Sender>;

    fn packetize(
        &mut self,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized>;

    async fn send(
        sender: &mut Self::Sender,
        packetized: Self::Packetized,
    ) -> eros::Result<SentBytes>;
}
