use super::*;

impl CaptureWorker {
    pub(crate) async fn spawn_capture_only<Capturer, State, EventReporter>(
        capture_source_id: CaptureSourceId,
        screen_capturer_state_constructor: impl FnOnce() -> eros::Result<State> + Send + 'static,
        event_reporter: EventReporter,
    ) -> eros::Result<CaptureWorkerHandle<Capturer>>
    where
        Capturer: ScreenCapturer + MetricsRecorder + From<(CaptureSourceId, State)> + 'static,
        EventReporter: HostEventReporter,
    {
        Self::spawn_with_frame_slots(
            capture_source_id,
            screen_capturer_state_constructor,
            HashMap::new(),
            event_reporter,
        )
        .await
    }
}
