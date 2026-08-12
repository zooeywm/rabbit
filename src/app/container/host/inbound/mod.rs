#[cfg(feature = "test-ui")]
mod capture_only;
mod capture_source_runtime;

pub(super) use capture_source_runtime::CaptureSourceRuntime;

use eros::Context;

use crate::{
    app::container::{
        host::{
            CapturedFrameFor, EncodedBufferFor, EncoderInputFor, HostContainer,
            HostStreamPipelineFor,
            inbound_port::HostApplication,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                HostEventReporter, MetricsRecorder,
            },
        },
        host_stream_pipeline::{
            inbound::HostStreamPipelineWorker,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
        network::inbound::EncodedUnitSender,
        screen_capture::inbound::CaptureWorker,
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    HostStreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
        + MetricsRecorder,
    EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
{
    fn compose_host_stream_pipeline_states(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<(
        CvtMgrSt::EncoderFrameConverterState,
        EcdMgrSt::VideoEncoderState,
    )>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, EcdMgrSt> {
        let encoder_frame_converter_state_constructor =
            self.compose_encoder_frame_converter_state();
        let video_encoder_state_constructor = self.compose_video_encoder_state();

        move || {
            Ok((
                encoder_frame_converter_state_constructor()?,
                video_encoder_state_constructor()?,
            ))
        }
    }

    async fn start_stream<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtMgrSt, EcdMgrSt>>,
        event_reporter: EventReporter,
    ) -> eros::Result<StreamId> {
        let stream_id = StreamId::new(self.next_stream_id);
        let next_stream_id = self
            .next_stream_id
            .checked_add(1)
            .with_context(|| "Stream ID space is exhausted")?;

        let screen_capturer_state_constructor = if self
            .capture_source_runtimes
            .contains_key(&capture_source_id)
        {
            None
        } else {
            Some(self.compose_screen_capturer_state(capture_source_id))
        };

        let host_stream_pipeline_states_constructor = self.compose_host_stream_pipeline_states();
        let host_stream_pipeline_handle = HostStreamPipelineWorker::spawn(
            capture_source_id,
            stream_id,
            host_stream_pipeline_states_constructor,
            encoded_unit_sender,
            event_reporter.clone(),
        )
        .await?;

        if let Some(screen_capturer_state_constructor) = screen_capturer_state_constructor {
            let capture_worker_handle =
                match CaptureWorker::spawn::<CapMgrSt::ScreenCapturer, _, _>(
                    capture_source_id,
                    screen_capturer_state_constructor,
                    stream_id,
                    host_stream_pipeline_handle.frame_slot(),
                    event_reporter,
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

        self.next_stream_id = next_stream_id;

        Ok(stream_id)
    }

    async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
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

        remove_result?;
        shutdown_result
    }

    async fn remove_stream_after_host_pipeline_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        let (pipeline_result, should_remove_capture_source) = {
            let capture_source_runtime = self
                .capture_source_runtimes
                .get_mut(&capture_source_id)
                .with_context(|| "Capture source does not exist")?;
            let pipeline_result = capture_source_runtime
                .remove_stream_after_host_pipeline_exit(stream_id)
                .await;

            (pipeline_result, capture_source_runtime.is_empty())
        };

        let capture_shutdown_result = if should_remove_capture_source {
            let capture_source_runtime = self
                .capture_source_runtimes
                .remove(&capture_source_id)
                .with_context(|| "Capture source disappeared before shutdown")?;

            capture_source_runtime.shutdown().await
        } else {
            Ok(())
        };

        pipeline_result?;
        capture_shutdown_result
    }

    async fn handle_capture_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> Option<eros::Result<()>> {
        let capture_source_runtime = self.capture_source_runtimes.remove(&capture_source_id)?;

        Some(
            match capture_source_runtime
                .shutdown_after_capture_worker_exit()
                .await
            {
                Ok(()) => Err(eros::error!("Capture worker exited unexpectedly")),
                Err(error) => Err(error),
            },
        )
    }

    async fn handle_host_stream_pipeline_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> Option<eros::Result<()>> {
        let is_current_worker = self
            .capture_source_runtimes
            .get(&capture_source_id)
            .is_some_and(|runtime| runtime.contains_stream(stream_id));

        if !is_current_worker {
            return None;
        }

        Some(
            match self
                .remove_stream_after_host_pipeline_exit(capture_source_id, stream_id)
                .await
            {
                Ok(()) => Err(eros::error!(
                    "Host stream pipeline worker exited unexpectedly"
                )),
                Err(error) => Err(error),
            },
        )
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> HostApplication for HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    HostStreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
        + MetricsRecorder,
    EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
{
    type EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>;

    async fn start_stream<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        encoded_unit_sender: EncodedUnitSender<Self::EncodedBuffer>,
        event_reporter: EventReporter,
    ) -> eros::Result<StreamId> {
        HostContainer::start_stream(self, capture_source_id, encoded_unit_sender, event_reporter)
            .await
    }

    async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        HostContainer::remove_stream(self, stream_id).await
    }

    async fn handle_capture_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> Option<eros::Result<()>> {
        HostContainer::handle_capture_worker_exit(self, capture_source_id).await
    }

    async fn handle_host_stream_pipeline_worker_exit(
        &mut self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> Option<eros::Result<()>> {
        HostContainer::handle_host_stream_pipeline_worker_exit(self, capture_source_id, stream_id)
            .await
    }

    #[cfg(feature = "test-ui")]
    async fn start_capture_only<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        event_reporter: EventReporter,
    ) -> eros::Result<()> {
        HostContainer::start_capture_only(self, capture_source_id, event_reporter).await
    }

    #[cfg(feature = "test-ui")]
    async fn stop_capture_only(&mut self, capture_source_id: CaptureSourceId) -> eros::Result<()> {
        HostContainer::stop_capture_only(self, capture_source_id).await
    }

    async fn shutdown(self) -> eros::Result<()> {
        HostContainer::shutdown(self).await
    }
}
