#[cfg(feature = "test-ui")]
mod capture_only;
mod capture_source_runtime;

pub(super) use capture_source_runtime::CaptureSourceRuntime;
use eros::Context;

use crate::{
    app::{
        container::{
            root::{
                AppContainer, CapturedFrameFor, EncodedBufferFor, EncoderInputFor,
                StreamPipelineFor,
                outbound_port::{
                    CapturerManager, CapturerManagerStateSpec, ConverterManager,
                    ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                    MetricsRecorder,
                },
            },
            screen_capture::inbound::CaptureWorker,
            stream_pipeline::{
                inbound::StreamPipelineWorker,
                outbound_port::{EncoderFrameConverter, VideoEncoder},
            },
            transporter::inbound::EncodedUnitSender,
        },
        runtime::AppMessage,
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

impl<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt> AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
        + MetricsRecorder,
    EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
{
    fn compose_stream_pipeline_states(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<(
        CvtMgrSt::EncoderFrameConverterState,
        EcdMgrSt::VideoEncoderState,
    )>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt> {
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

    pub(crate) async fn start_stream(
        &mut self,
        capture_source_id: CaptureSourceId,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtMgrSt, EcdMgrSt>>,
        app_message_sender: &flume::Sender<AppMessage>,
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

        let stream_pipeline_states_constructor = self.compose_stream_pipeline_states();
        let stream_pipeline_handle = StreamPipelineWorker::spawn(
            capture_source_id,
            stream_id,
            stream_pipeline_states_constructor,
            encoded_unit_sender,
            app_message_sender.clone(),
        )
        .await?;

        if let Some(screen_capturer_state_constructor) = screen_capturer_state_constructor {
            let capture_worker_handle = match CaptureWorker::spawn::<CapMgrSt::ScreenCapturer, _>(
                capture_source_id,
                screen_capturer_state_constructor,
                stream_id,
                stream_pipeline_handle.frame_slot(),
                app_message_sender.clone(),
            )
            .await
            {
                Ok(capture_worker_handle) => capture_worker_handle,
                Err(error) => {
                    let _ = stream_pipeline_handle.shutdown().await;
                    return Err(error);
                }
            };

            self.capture_source_runtimes.insert(
                capture_source_id,
                CaptureSourceRuntime::new(capture_worker_handle, stream_id, stream_pipeline_handle),
            );
        } else {
            self.capture_source_runtimes
                .get_mut(&capture_source_id)
                .with_context(|| "Capture source disappeared while adding stream")?
                .add_stream(stream_id, stream_pipeline_handle)
                .await?;
        }

        self.next_stream_id = next_stream_id;

        Ok(stream_id)
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
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

    async fn remove_stream_after_pipeline_exit(
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
                .remove_stream_after_pipeline_exit(stream_id)
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

    pub(crate) async fn run(
        mut self,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtMgrSt, EcdMgrSt>>,
        app_message_sender: flume::Sender<AppMessage>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> eros::Result<()> {
        loop {
            match message_receiver.recv_async().await {
                #[cfg(feature = "test-ui")]
                Ok(AppMessage::CaptureOnly(message)) => {
                    self.handle_capture_only_message(message, &app_message_sender)
                        .await;
                }
                Ok(AppMessage::StartStream {
                    capture_source_id,
                    response_sender,
                }) => {
                    let _ = response_sender.send(
                        self.start_stream(
                            capture_source_id,
                            encoded_unit_sender.clone(),
                            &app_message_sender,
                        )
                        .await,
                    );
                }
                Ok(AppMessage::RemoveStream {
                    stream_id,
                    response_sender,
                }) => {
                    let _ = response_sender.send(self.remove_stream(stream_id).await);
                }
                Ok(AppMessage::CaptureWorkerExited { capture_source_id }) => {
                    let Some(capture_source_runtime) =
                        self.capture_source_runtimes.remove(&capture_source_id)
                    else {
                        continue;
                    };

                    let failure = capture_source_runtime
                        .shutdown_after_capture_worker_exit()
                        .await
                        .err()
                        .unwrap_or_else(|| eros::error!("Capture worker exited unexpectedly"));

                    let _ = self.shutdown().await;
                    return Err(failure);
                }
                Ok(AppMessage::StreamPipelineWorkerExited {
                    capture_source_id,
                    stream_id,
                }) => {
                    let is_current_worker = self
                        .capture_source_runtimes
                        .get(&capture_source_id)
                        .is_some_and(|runtime| runtime.contains_stream(stream_id));

                    if !is_current_worker {
                        continue;
                    }

                    let failure = self
                        .remove_stream_after_pipeline_exit(capture_source_id, stream_id)
                        .await
                        .err()
                        .unwrap_or_else(|| {
                            eros::error!("Stream pipeline worker exited unexpectedly")
                        });

                    let _ = self.shutdown().await;
                    return Err(failure);
                }
                Ok(AppMessage::TransporterWorkerExited) => {
                    let _ = self.shutdown().await;
                    eros::bail!("Transporter worker exited unexpectedly");
                }
                Ok(AppMessage::Shutdown) | Err(_) => break,
            }
        }

        self.shutdown().await
    }
}
