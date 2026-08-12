use super::*;

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
{
    pub(super) async fn start_capture_only<EventReporter: HostEventReporter>(
        &mut self,
        capture_source_id: CaptureSourceId,
        event_reporter: EventReporter,
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
            CaptureWorker::spawn_capture_only::<CapMgrSt::ScreenCapturer, _, _>(
                capture_source_id,
                screen_capturer_state_constructor,
                event_reporter,
            )
            .await?;

        self.capture_source_runtimes.insert(
            capture_source_id,
            CaptureSourceRuntime::capture_only(capture_worker_handle),
        );

        Ok(())
    }

    pub(super) async fn stop_capture_only(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<()> {
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
}
