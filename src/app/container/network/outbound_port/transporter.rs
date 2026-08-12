use crate::{
    app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit,
    domain::stream::models::vo::StreamId,
};

pub(crate) trait Transporter {
    type EncodedBuffer;
    type Packetized;

    fn packetize(
        &mut self,
        stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized>;

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()>;
}
