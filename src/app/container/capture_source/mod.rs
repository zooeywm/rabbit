mod inbound;
mod screen_capturer_container;
mod worker;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{app::container::StreamPipelineWorkerHandle, domain::stream::models::vo::StreamId};

pub(crate) use screen_capturer_container::ScreenCapturerContainer;
pub(crate) use worker::{CaptureWorker, CaptureWorkerHandle};

pub(crate) struct CaptureSourceContainer<Frame: Clone + Send + 'static> {
    capture_worker_handle: CaptureWorkerHandle<Frame>,
    stream_pipeline_handles: HashMap<StreamId, StreamPipelineWorkerHandle<Frame>>,
}

impl<Frame: Clone + Send + 'static> CaptureSourceContainer<Frame> {
    pub(crate) fn new(
        capture_worker_handle: CaptureWorkerHandle<Frame>,
        initial_stream_id: StreamId,
        initial_stream_pipeline_handle: StreamPipelineWorkerHandle<Frame>,
    ) -> Self {
        Self {
            capture_worker_handle,
            stream_pipeline_handles: HashMap::from([(
                initial_stream_id,
                initial_stream_pipeline_handle,
            )]),
        }
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            capture_worker_handle,
            stream_pipeline_handles,
        } = self;

        for stream_pipeline_handle in stream_pipeline_handles.values() {
            stream_pipeline_handle.close();
        }

        let mut first_error = capture_worker_handle.shutdown().await.err();

        for stream_pipeline_handle in stream_pipeline_handles.into_values() {
            if let Err(error) = stream_pipeline_handle.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
