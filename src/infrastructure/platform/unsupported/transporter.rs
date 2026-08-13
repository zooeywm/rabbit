use std::convert::Infallible;

use crate::{
    app::container::{
        host_stream_pipeline::outbound_port::EncodedVideoUnit,
        network::outbound_port::{
            NetworkState, SentBytes, TransporterClientSide, TransporterHostSide,
        },
    },
    domain::stream::models::{StreamRequest, vo::StreamId},
};

#[derive(kudi::DepInj)]
#[target(UnsupportedTransporterImpl)]
pub(crate) struct UnsupportedTransporterState {
    never: Infallible,
}

impl UnsupportedTransporterState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Transporter is not implemented on this platform")
    }
}

impl NetworkState for UnsupportedTransporterState {
    type Config = ();
    type EncodedBuffer = Infallible;
    type ClientInput = Infallible;

    fn new((): Self::Config) -> eros::Result<Self> {
        Self::new()
    }
}

impl<Deps> TransporterHostSide for UnsupportedTransporterImpl<Deps> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;
    type Host = Infallible;
    type RequestReceiver = Infallible;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver> {
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

    async fn receive_request(
        receiver: &mut Self::RequestReceiver,
    ) -> eros::Result<Option<StreamRequest>> {
        match *receiver {}
    }
}

impl<Deps> TransporterClientSide for UnsupportedTransporterImpl<Deps> {
    type Receiver = Infallible;
    type RequestSender = Infallible;
    type Received = Infallible;
    type Depacketized = Infallible;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    async fn receive(_receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        eros::bail!("Transporter is not implemented on this platform")
    }

    async fn send_request(
        sender: &mut Self::RequestSender,
        _request: StreamRequest,
    ) -> eros::Result<()> {
        match *sender {}
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
