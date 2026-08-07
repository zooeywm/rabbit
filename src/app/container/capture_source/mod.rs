mod inbound;
mod screen_capturer_container;
mod worker;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{app::container::StreamPipelineWorkerHandle, domain::stream::models::vo::StreamId};

pub(crate) use screen_capturer_container::ScreenCapturerContainer;
pub(crate) use worker::{CaptureWorker, CaptureWorkerHandle};

pub(crate) struct CaptureSourceContainer<Frame: Clone + Send + 'static> {
    capture_worker: CaptureWorkerHandle<Frame>,
    stream_pipelines: HashMap<StreamId, StreamPipelineWorkerHandle<Frame>>,
}

impl<Frame: Clone + Send + 'static> CaptureSourceContainer<Frame> {
    pub(crate) fn new(
        capture_worker: CaptureWorkerHandle<Frame>,
        initial_stream_id: StreamId,
        initial_stream_pipeline: StreamPipelineWorkerHandle<Frame>,
    ) -> Self {
        Self {
            capture_worker,
            stream_pipelines: HashMap::from([(initial_stream_id, initial_stream_pipeline)]),
        }
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            capture_worker,
            stream_pipelines,
        } = self;

        for stream_pipeline in stream_pipelines.values() {
            stream_pipeline.close();
        }

        let mut first_error = capture_worker.shutdown().await.err();

        for stream_pipeline in stream_pipelines.into_values() {
            if let Err(error) = stream_pipeline.shutdown().await
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
