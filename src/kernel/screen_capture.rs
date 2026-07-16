use crate::kernel::screen_manager::ScreenId;

/// A composed physical-screen frame and any recoverable source-layer issues.
pub struct CapturedFrame<Buffer, Issue> {
    pub buffer: Buffer,
    pub issues: Vec<Issue>,
}

/// Provides subscriptions to shared physical-screen capture sources.
pub trait ScreenCaptureManager {
    type Buffer;
    type Issue;
    type Subscription: futures_core::Stream<
        Item = eros::Result<CapturedFrame<Self::Buffer, Self::Issue>>,
    >;

    fn subscribe(&mut self, screen_id: &ScreenId) -> eros::Result<Self::Subscription>;
}
