pub(crate) enum CaptureLoopAction {
    Continue { consumer_count: usize },
    Stop,
}

pub(crate) trait ScreenCapturerControl: Send + 'static {
    fn wake(&self) -> eros::Result<()>;
}

impl ScreenCapturerControl for std::convert::Infallible {
    fn wake(&self) -> eros::Result<()> {
        match *self {}
    }
}

pub(crate) trait ScreenCapturer {
    type CapturedFrame: Clone + Send + 'static;
    type Control: ScreenCapturerControl;

    fn control(&self) -> eros::Result<Self::Control>;

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
