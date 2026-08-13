use std::convert::Infallible;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{SentBytes, TransporterClientSide, TransporterHostSide},
    },
    domain::stream::models::vo::StreamId,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedTransporterImpl)]
pub(crate) struct UnsupportedTransporterState {
    never: Infallible,
}

impl<Deps> TransporterHostSide for UnsupportedTransporterImpl<Deps> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;
    type Host = Infallible;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    fn packetize(
        _host: &mut Self::Host,
        _stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        match unit.data {}
    }

    async fn send(_host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes> {
        match packetized {}
    }
}

impl<Deps> TransporterClientSide for UnsupportedTransporterImpl<Deps> {
    type Receiver = Infallible;
    type Received = Infallible;
    type Depacketized = Infallible;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    async fn receive(_receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(StreamId, Self::Depacketized)> {
        match received {}
    }

    fn request_video_refresh(&mut self, _stream_id: StreamId) -> eros::Result<()> {
        eros::bail!("Transporter is not implemented on this platform")
    }
}
