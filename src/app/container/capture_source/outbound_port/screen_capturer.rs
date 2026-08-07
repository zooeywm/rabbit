pub(crate) enum CaptureLoopAction {
    Continue { consumer_count: usize },
    Stop,
}

pub(crate) trait ScreenCapturer {
    type CapturedFrame: Clone + Send + 'static;

    fn run<OnStarted, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>;
}
