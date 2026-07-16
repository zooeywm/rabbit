use crate::kernel::screen_manager::ScreenId;

/// Provides subscriptions to shared physical-screen capture sources.
pub trait ScreenCaptureManager {
    type Frame;
    type Subscription: futures_core::Stream<Item = eros::Result<Self::Frame>>;

    fn subscribe(&mut self, screen_id: &ScreenId) -> eros::Result<Self::Subscription>;
}
