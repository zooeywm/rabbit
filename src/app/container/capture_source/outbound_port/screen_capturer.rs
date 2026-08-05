/// Captures frames from one physical screen capture source.
///
/// This port describes capture capability only. Threading, asynchronous
/// execution, and worker lifecycle are controlled by the container.
pub trait ScreenCapturer {
    /// The frame type produced by this capturer.
    type CapturedFrame;

    /// Captures the next available frame.
    fn capture_next(&mut self) -> eros::Result<Self::CapturedFrame>;
}
