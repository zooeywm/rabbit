use eros::Context;

use crate::{
    app::container::{CaptureSourceContainer, StreamPipelineWorkerHandle},
    domain::stream::models::vo::StreamId,
};

impl<Frame: Clone + Send + 'static> CaptureSourceContainer<Frame> {
    pub(crate) fn contains_stream(&self, stream_id: StreamId) -> bool {
        self.stream_pipelines.contains_key(&stream_id)
    }

    pub(crate) async fn add_stream(
        &mut self,
        stream_id: StreamId,
        stream_pipeline: StreamPipelineWorkerHandle<Frame>,
    ) -> eros::Result<()> {
        if self.stream_pipelines.contains_key(&stream_id) {
            stream_pipeline.shutdown().await?;
            eros::bail!("Stream pipeline already exists");
        }

        if let Err(error) = self
            .capture_worker
            .add_stream(stream_id, stream_pipeline.slot())
            .await
        {
            let _ = stream_pipeline.shutdown().await;
            return Err(error);
        }

        self.stream_pipelines.insert(stream_id, stream_pipeline);

        Ok(())
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<bool> {
        let stream_pipeline = self
            .stream_pipelines
            .remove(&stream_id)
            .with_context(|| "Stream pipeline does not exist")?;

        stream_pipeline.close();

        let remove_result = self.capture_worker.remove_stream(stream_id).await;
        let shutdown_result = stream_pipeline.shutdown().await;

        remove_result?;
        shutdown_result?;

        Ok(self.stream_pipelines.is_empty())
    }
}
