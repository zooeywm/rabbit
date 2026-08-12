use std::convert::Infallible;

use crate::domain::stream::models::vo::StreamId;

pub(crate) trait ClientApplication {
    type NetworkInput: Send + 'static;

    fn handle_network_input(
        &mut self,
        stream_id: StreamId,
        input: Self::NetworkInput,
    ) -> eros::Result<()>;

    async fn shutdown(self) -> eros::Result<()>;
}

impl ClientApplication for super::ClientContainer<(), ()> {
    type NetworkInput = Infallible;

    fn handle_network_input(
        &mut self,
        _stream_id: StreamId,
        input: Self::NetworkInput,
    ) -> eros::Result<()> {
        match input {}
    }

    async fn shutdown(self) -> eros::Result<()> {
        Ok(())
    }
}
