mod capture_source_runtime;

pub(super) use capture_source_runtime::CaptureSourceRuntime;

use eros::Context;

use crate::{
    app::container::{
        host::{
            CapturedFrameFor, EncodedBufferFor, EncoderInputFor, HostContainer,
            HostStreamPipelineFor,
            outbound_port::{
                EncoderFrameConverterState, MetricsRecorder, ScreenCapturerState, VideoEncoderState,
            },
        },
        host_stream_pipeline::{
            inbound::HostStreamPipelineWorker,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
        screen_capture::{
            ScreenCaptureContainer, inbound::CaptureWorker, outbound_port::ScreenCapturer,
        },
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

impl<CapSt, CvtSt, EcdSt> HostContainer<CapSt, CvtSt, EcdSt>
where
    CapSt: ScreenCapturerState,
    CvtSt: EncoderFrameConverterState,
    EcdSt: VideoEncoderState,
    ScreenCaptureContainer<CapSt>: ScreenCapturer + MetricsRecorder + 'static,
    HostStreamPipelineFor<CvtSt, EcdSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtSt, EcdSt>>
        + MetricsRecorder,
    EncodedBufferFor<CvtSt, EcdSt>: Send + 'static,
{
    pub(in crate::app) async fn start_stream(
        &mut self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        let needs_capture_worker = !self
            .capture_source_runtimes
            .contains_key(&capture_source_id);

        let host_stream_pipeline_handle = HostStreamPipelineWorker::spawn(
            capture_source_id,
            stream_id,
            self.encoded_unit_sender.clone(),
        )
        .await?;

        if needs_capture_worker {
            let capture_worker_handle =
                match CaptureWorker::spawn::<ScreenCaptureContainer<CapSt>, CapSt>(
                    capture_source_id,
                    stream_id,
                    host_stream_pipeline_handle.frame_slot(),
                )
                .await
                {
                    Ok(capture_worker_handle) => capture_worker_handle,
                    Err(error) => {
                        let _ = host_stream_pipeline_handle.shutdown().await;
                        return Err(error);
                    }
                };

            self.capture_source_runtimes.insert(
                capture_source_id,
                CaptureSourceRuntime::new(
                    capture_worker_handle,
                    stream_id,
                    host_stream_pipeline_handle,
                ),
            );
        } else {
            self.capture_source_runtimes
                .get_mut(&capture_source_id)
                .with_context(|| "Capture source disappeared while adding stream")?
                .add_stream(stream_id, host_stream_pipeline_handle)
                .await?;
        }

        Ok(())
    }

    pub(in crate::app) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let capture_source_id = self
            .capture_source_runtimes
            .iter()
            .find_map(|(capture_source_id, capture_source)| {
                capture_source
                    .contains_stream(stream_id)
                    .then_some(*capture_source_id)
            })
            .with_context(|| "Stream does not exist")?;

        let (remove_result, should_remove_capture_source) = {
            let capture_source_runtime =
                self.capture_source_runtimes
                    .get_mut(&capture_source_id)
                    .with_context(|| "Capture source disappeared while removing stream")?;
            let remove_result = capture_source_runtime.remove_stream(stream_id).await;

            (remove_result, capture_source_runtime.is_empty())
        };

        let shutdown_result = if should_remove_capture_source {
            let capture_source_runtime = self
                .capture_source_runtimes
                .remove(&capture_source_id)
                .with_context(|| "Capture source disappeared before shutdown")?;

            capture_source_runtime.shutdown().await
        } else {
            Ok(())
        };

        shutdown_result?;
        remove_result
    }
}
