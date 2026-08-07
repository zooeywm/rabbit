use std::sync::Arc;

pub(crate) enum CaptureLoopAction {
    Continue { consumer_count: usize },
    Stop,
}

pub(crate) trait ScreenCapturerControl: Send + Sync + 'static {
    fn wake(&self) -> eros::Result<()>;
}

pub(crate) trait ScreenCapturer {
    type CapturedFrame: Clone + Send + 'static;

    fn control(&self) -> eros::Result<Arc<dyn ScreenCapturerControl>>;

    /// `control.wake()` must interrupt any pending frame wait and cause
    /// `on_control` to run even when no new frame is available.
    fn run<OnStarted, OnControl, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        on_control: OnControl,
        on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>;
}
