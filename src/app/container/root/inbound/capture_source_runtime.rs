use std::collections::HashMap;

use eros::Context;

use crate::{
    app::container::{
        screen_capture::{inbound::CaptureWorkerHandle, outbound_port::ScreenCapturer},
        stream_pipeline::inbound::StreamPipelineWorkerHandle,
    },
    domain::stream::models::vo::StreamId,
};

pub(in crate::app::container::root) struct CaptureSourceRuntime<Capturer: ScreenCapturer> {
    capture_worker_handle: CaptureWorkerHandle<Capturer>,
    stream_pipeline_handles: HashMap<StreamId, StreamPipelineWorkerHandle<Capturer::CapturedFrame>>,
}

impl<Capturer: ScreenCapturer> CaptureSourceRuntime<Capturer> {
    pub(in crate::app::container::root) fn new(
        capture_worker_handle: CaptureWorkerHandle<Capturer>,
        initial_stream_id: StreamId,
        initial_stream_pipeline_handle: StreamPipelineWorkerHandle<Capturer::CapturedFrame>,
    ) -> Self {
        Self {
            capture_worker_handle,
            stream_pipeline_handles: HashMap::from([(
                initial_stream_id,
                initial_stream_pipeline_handle,
            )]),
        }
    }

    pub(in crate::app::container::root) async fn shutdown(self) -> eros::Result<()> {
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

    pub(in crate::app::container::root) async fn shutdown_after_capture_worker_exit(
        self,
    ) -> eros::Result<()> {
        let Self {
            capture_worker_handle,
            stream_pipeline_handles,
        } = self;

        for stream_pipeline_handle in stream_pipeline_handles.values() {
            stream_pipeline_handle.close();
        }

        let mut first_error = capture_worker_handle.join().await.err();

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

    pub(in crate::app::container::root) fn contains_stream(&self, stream_id: StreamId) -> bool {
        self.stream_pipeline_handles.contains_key(&stream_id)
    }

    pub(in crate::app::container::root) fn is_empty(&self) -> bool {
        self.stream_pipeline_handles.is_empty()
    }

    pub(in crate::app::container::root) async fn add_stream(
        &mut self,
        stream_id: StreamId,
        stream_pipeline_handle: StreamPipelineWorkerHandle<Capturer::CapturedFrame>,
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

    pub(in crate::app::container::root) async fn remove_stream(
        &mut self,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        self.stream_pipeline_handles
            .get(&stream_id)
            .with_context(|| "Stream pipeline does not exist")?;

        self.capture_worker_handle.remove_stream(stream_id).await?;

        let stream_pipeline_handle = self
            .stream_pipeline_handles
            .remove(&stream_id)
            .with_context(|| "Stream pipeline disappeared while removing stream")?;

        stream_pipeline_handle.shutdown().await
    }

    pub(in crate::app::container::root) async fn remove_stream_after_pipeline_exit(
        &mut self,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        let stream_pipeline_handle = self
            .stream_pipeline_handles
            .remove(&stream_id)
            .with_context(|| "Stream pipeline does not exist")?;

        stream_pipeline_handle.close();

        let _ = self.capture_worker_handle.remove_stream(stream_id).await;

        stream_pipeline_handle.shutdown().await
    }
}
