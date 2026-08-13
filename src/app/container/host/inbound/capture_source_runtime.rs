use std::collections::HashMap;

use eros::Context;

use crate::{
    app::container::{
        host_stream_pipeline::inbound::HostStreamPipelineWorkerHandle,
        screen_capture::{inbound::CaptureWorkerHandle, outbound_port::ScreenCapturer},
    },
    domain::stream::models::vo::StreamId,
};

pub(in crate::app::container::host) struct CaptureSourceRuntime<Capturer: ScreenCapturer> {
    capture_worker_handle: CaptureWorkerHandle<Capturer>,
    host_stream_pipeline_handles:
        HashMap<StreamId, HostStreamPipelineWorkerHandle<Capturer::CapturedFrame>>,
}

impl<Capturer: ScreenCapturer> CaptureSourceRuntime<Capturer> {
    pub(in crate::app::container::host) fn new(
        capture_worker_handle: CaptureWorkerHandle<Capturer>,
        initial_stream_id: StreamId,
        initial_host_stream_pipeline_handle: HostStreamPipelineWorkerHandle<
            Capturer::CapturedFrame,
        >,
    ) -> Self {
        Self {
            capture_worker_handle,
            host_stream_pipeline_handles: HashMap::from([(
                initial_stream_id,
                initial_host_stream_pipeline_handle,
            )]),
        }
    }

    pub(in crate::app::container::host) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            capture_worker_handle,
            host_stream_pipeline_handles,
        } = self;

        for host_stream_pipeline_handle in host_stream_pipeline_handles.values() {
            host_stream_pipeline_handle.close();
        }

        let mut first_error = capture_worker_handle.shutdown().await.err();

        for host_stream_pipeline_handle in host_stream_pipeline_handles.into_values() {
            if let Err(error) = host_stream_pipeline_handle.shutdown().await
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

    pub(in crate::app::container::host) fn contains_stream(&self, stream_id: StreamId) -> bool {
        self.host_stream_pipeline_handles.contains_key(&stream_id)
    }

    pub(in crate::app::container::host) fn is_empty(&self) -> bool {
        self.host_stream_pipeline_handles.is_empty()
    }

    pub(in crate::app::container::host) async fn add_stream(
        &mut self,
        stream_id: StreamId,
        host_stream_pipeline_handle: HostStreamPipelineWorkerHandle<Capturer::CapturedFrame>,
    ) -> eros::Result<()> {
        if self.host_stream_pipeline_handles.contains_key(&stream_id) {
            host_stream_pipeline_handle.shutdown().await?;
            eros::bail!("Host stream pipeline already exists");
        }

        if let Err(error) = self
            .capture_worker_handle
            .add_stream(stream_id, host_stream_pipeline_handle.frame_slot())
            .await
        {
            let _ = host_stream_pipeline_handle.shutdown().await;
            return Err(error);
        }

        self.host_stream_pipeline_handles
            .insert(stream_id, host_stream_pipeline_handle);

        Ok(())
    }

    pub(in crate::app::container::host) async fn remove_stream(
        &mut self,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        self.host_stream_pipeline_handles
            .get(&stream_id)
            .with_context(|| "Host stream pipeline does not exist")?;

        let capture_result = self.capture_worker_handle.remove_stream(stream_id).await;

        let host_stream_pipeline_handle = self
            .host_stream_pipeline_handles
            .remove(&stream_id)
            .with_context(|| "Host stream pipeline disappeared while removing stream")?;

        let pipeline_result = host_stream_pipeline_handle.shutdown().await;

        pipeline_result?;
        capture_result
    }
}
