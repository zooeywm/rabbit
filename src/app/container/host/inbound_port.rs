use crate::{
    app::container::{host::outbound_port::HostEventReporter, network::inbound::EncodedUnitSender},
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

pub(crate) trait HostApplication {
    type EncodedBuffer: Send + 'static;

    async fn start_stream<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        encoded_unit_sender: EncodedUnitSender<Self::EncodedBuffer>,
        event_reporter: EventReporter,
    ) -> eros::Result<StreamId>;

    async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()>;

    async fn handle_capture_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> Option<eros::ErrorUnion>;

    async fn handle_host_stream_pipeline_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> Option<eros::ErrorUnion>;

    #[cfg(feature = "test-ui")]
    async fn start_capture_only<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        event_reporter: EventReporter,
    ) -> eros::Result<()>;

    #[cfg(feature = "test-ui")]
    async fn stop_capture_only(&mut self, capture_source_id: CaptureSourceId) -> eros::Result<()>;

    async fn shutdown(self) -> eros::Result<()>;
}
