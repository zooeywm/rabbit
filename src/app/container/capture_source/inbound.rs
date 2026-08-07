use eros::Context;

use crate::{
    app::container::{CaptureSourceContainer, StreamPipelineWorkerHandle},
    domain::stream::models::vo::StreamId,
};

impl<Frame: Clone + Send + 'static> CaptureSourceContainer<Frame> {
    pub(crate) fn contains_stream(&self, stream_id: StreamId) -> bool {
        self.stream_pipeline_handles.contains_key(&stream_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.stream_pipeline_handles.is_empty()
    }

    pub(crate) async fn add_stream(
        &mut self,
        stream_id: StreamId,
        stream_pipeline_handle: StreamPipelineWorkerHandle<Frame>,
    ) -> eros::Result<()> {
        if self.stream_pipeline_handles.contains_key(&stream_id) {
            stream_pipeline_handle.shutdown().await?;
            eros::bail!("Stream pipeline already exists");
        }

        if let Err(error) = self
            .capture_worker_handle
            .add_stream(stream_id, stream_pipeline_handle.frame_slot())
            .await
        {
            let _ = stream_pipeline_handle.shutdown().await;
            return Err(error);
        }

        self.stream_pipeline_handles
            .insert(stream_id, stream_pipeline_handle);

        Ok(())
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let stream_pipeline_handle = self
            .stream_pipeline_handles
            .remove(&stream_id)
            .with_context(|| "Stream pipeline does not exist")?;

        stream_pipeline_handle.close();

        let remove_result = self.capture_worker_handle.remove_stream(stream_id).await;
        let shutdown_result = stream_pipeline_handle.shutdown().await;

        remove_result?;
        shutdown_result?;

        Ok(())
    }
}
