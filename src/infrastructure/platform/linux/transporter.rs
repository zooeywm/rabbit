use std::convert::Infallible;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{SentBytes, TransporterClientSide, TransporterHostSide},
    },
    domain::stream::models::vo::StreamId,
};

#[derive(kudi::DepInj)]
#[target(LinuxTransporterImpl)]
pub(crate) struct LinuxTransporterState {
    never: Infallible,
}

impl<Deps> TransporterHostSide for LinuxTransporterImpl<Deps> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;
    type Host = Infallible;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        eros::bail!("Linux transporter is not implemented")
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

impl<Deps> TransporterClientSide for LinuxTransporterImpl<Deps> {
    type Receiver = Infallible;
    type Received = Infallible;
    type Depacketized = Infallible;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        eros::bail!("Linux transporter is not implemented")
    }

    async fn receive(_receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        eros::bail!("Linux transporter is not implemented")
    }

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(StreamId, Self::Depacketized)> {
        match received {}
    }

    fn request_video_refresh(&mut self, _stream_id: StreamId) -> eros::Result<()> {
        eros::bail!("Linux transporter is not implemented")
    }
}
