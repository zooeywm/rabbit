use crate::app::runtime::capture_only::CaptureOnlyMessage;

use super::*;

impl<CapMgrSt, CvtMgrSt, EcdMgrSt, PktMgrSt> AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    PktMgrSt: PacketizerManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>
        + PacketizerManager<State = PktMgrSt>,
    StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
        + MetricsRecorder,
    PacketizerFor<PktMgrSt>: Packetizer<EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>>,
{
    async fn start_capture_only(
        &mut self,
        capture_source_id: CaptureSourceId,
        app_message_sender: &flume::Sender<AppMessage>,
    ) -> eros::Result<()> {
        if self
            .capture_source_runtimes
            .contains_key(&capture_source_id)
        {
            eros::bail!("Capture source already exists");
        }

        let screen_capturer_state_constructor =
            self.compose_screen_capturer_state(capture_source_id);
        let capture_worker_handle =
            CaptureWorker::spawn_capture_only::<CapMgrSt::ScreenCapturer, _>(
                capture_source_id,
                screen_capturer_state_constructor,
                app_message_sender.clone(),
            )
            .await?;

        self.capture_source_runtimes.insert(
            capture_source_id,
            CaptureSourceRuntime::capture_only(capture_worker_handle),
        );

        Ok(())
    }

    async fn stop_capture_only(&mut self, capture_source_id: CaptureSourceId) -> eros::Result<()> {
        let capture_source_runtime = self
            .capture_source_runtimes
            .get(&capture_source_id)
            .with_context(|| "Capture source does not exist")?;

        if !capture_source_runtime.is_empty() {
            eros::bail!("Capture source has active streams");
        }

        let capture_source_runtime = self
            .capture_source_runtimes
            .remove(&capture_source_id)
            .with_context(|| "Capture source disappeared before shutdown")?;

        capture_source_runtime.shutdown().await
    }

    pub(super) async fn handle_capture_only_message(
        &mut self,
        message: CaptureOnlyMessage,
        app_message_sender: &flume::Sender<AppMessage>,
    ) {
        match message {
            CaptureOnlyMessage::Start {
                capture_source_id,
                response_sender,
            } => {
                let _ = response_sender.send(
                    self.start_capture_only(capture_source_id, app_message_sender)
                        .await,
                );
            }
            CaptureOnlyMessage::Stop {
                capture_source_id,
                response_sender,
            } => {
                let _ = response_sender.send(self.stop_capture_only(capture_source_id).await);
            }
        }
    }
}
