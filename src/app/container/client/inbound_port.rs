use crate::{
    app::container::{
        client::outbound_port::ClientEventReporter,
        client_stream_pipeline::{inbound::DecodeUnitSender, outbound_port::VideoDecodeUnit},
    },
    domain::stream::models::vo::StreamId,
};

pub(crate) trait ClientApplication {
    type NetworkInput: VideoDecodeUnit + Send + 'static;

    async fn start_stream(
        &mut self,
        stream_id: StreamId,
        event_reporter: impl ClientEventReporter,
    ) -> eros::Result<DecodeUnitSender<Self::NetworkInput>>;

    async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()>;

    async fn handle_client_stream_pipeline_worker_exit(
        &mut self,
        stream_id: StreamId,
    ) -> Option<eros::ErrorUnion>;

    async fn shutdown(self) -> eros::Result<()>;
}

impl ClientApplication for super::ClientContainer<(), ()> {
    type NetworkInput = std::convert::Infallible;

    async fn start_stream(
        &mut self,
        _stream_id: StreamId,
        _event_reporter: impl ClientEventReporter,
    ) -> eros::Result<DecodeUnitSender<Self::NetworkInput>> {
        eros::bail!("Client video pipeline is not implemented")
    }

    async fn remove_stream(&mut self, _stream_id: StreamId) -> eros::Result<()> {
        eros::bail!("Client video pipeline is not implemented")
    }

    async fn handle_client_stream_pipeline_worker_exit(
        &mut self,
        _stream_id: StreamId,
    ) -> Option<eros::ErrorUnion> {
        None
    }

    async fn shutdown(self) -> eros::Result<()> {
        Ok(())
    }
}
