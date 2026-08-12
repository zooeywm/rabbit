use crate::domain::stream::models::vo::StreamId;

pub(crate) trait ClientEventReporter: Clone + Send + 'static {
    fn report_client_stream_pipeline_worker_exited(&self, stream_id: StreamId);
}
