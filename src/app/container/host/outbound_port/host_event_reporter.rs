use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

pub(crate) trait HostEventReporter: Clone + Send + 'static {
    fn report_capture_worker_exited(&self, capture_source_id: CaptureSourceId);

    fn report_host_stream_pipeline_worker_exited(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    );
}
