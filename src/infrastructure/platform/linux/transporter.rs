use std::convert::Infallible;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::TransporterHostSide,
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

    fn packetize(
        &mut self,
        _stream_id: StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        match unit.data {}
    }

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()> {
        match packetized {}
    }
}
