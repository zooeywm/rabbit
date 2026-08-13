use crate::{
    app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit,
    domain::stream::models::{StreamRequest, vo::StreamId},
};

use super::SentBytes;

pub(crate) trait TransporterHostSide {
    type EncodedBuffer;
    type Packetized;
    type Host: 'static;
    type RequestReceiver: 'static;

    fn take_host(&mut self) -> eros::Result<Self::Host>;
    fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver>;

    fn packetize(
        host: &mut Self::Host,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized>;

    async fn send(host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes>;

    async fn receive_request(
        receiver: &mut Self::RequestReceiver,
    ) -> eros::Result<Option<StreamRequest>>;
}
