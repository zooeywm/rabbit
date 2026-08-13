use crate::{
    app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit,
    domain::stream::models::vo::StreamId,
};

use super::SentBytes;

pub(crate) trait TransporterHostSide {
    type EncodedBuffer;
    type Packetized;
    type Host: 'static;

    fn take_host(&mut self) -> eros::Result<Self::Host>;

    fn packetize(
        host: &mut Self::Host,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized>;

    async fn send(host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes>;
}
