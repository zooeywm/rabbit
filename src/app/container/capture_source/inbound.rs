use super::{super::stream_pipeline::StreamPipelineContainer, CaptureSourceContainer};
use crate::domain::stream::models::vo::StreamId;
use eros::Context;

impl<CapSt, CvtSt, EcdSt> CaptureSourceContainer<CapSt, CvtSt, EcdSt> {
    pub(crate) fn contains_stream(&self, stream_id: StreamId) -> bool {
        self.stream_pipelines.contains_key(&stream_id)
    }

    pub(crate) fn add_stream(
        &mut self,
        stream_id: StreamId,
        stream_pipeline: StreamPipelineContainer<CvtSt, EcdSt>,
    ) -> eros::Result<()> {
        if self.stream_pipelines.contains_key(&stream_id) {
            eros::bail!("Stream pipeline already exists");
        }

        self.stream_pipelines.insert(stream_id, stream_pipeline);

        Ok(())
    }

    pub(crate) fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<bool> {
        self.stream_pipelines
            .remove(&stream_id)
            .with_context(|| "Stream pipeline does not exist")?;

        Ok(self.stream_pipelines.is_empty())
    }
}
