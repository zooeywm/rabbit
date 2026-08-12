use std::convert::Infallible;

use crate::{
    app::container::{
        stream_pipeline::outbound_port::EncodedVideoUnit, transporter::outbound_port::Transporter,
    },
    domain::stream::models::vo::StreamId,
};

#[derive(kudi::DepInj)]
#[target(LinuxTransporterImpl)]
pub(crate) struct LinuxTransporterState {
    never: Infallible,
}

impl<Deps> Transporter for LinuxTransporterImpl<Deps> {
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
