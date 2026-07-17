use std::rc::Rc;

use crate::kernel::screen_manager::ScreenId;

/// A composed physical-screen frame and any recoverable source-layer issues.
#[derive(Debug)]
pub struct CapturedFrame<Buffer, Issue> {
    pub buffer: Buffer,
    pub issues: Vec<Issue>,
}

/// Provides subscriptions to shared physical-screen capture sources.
pub trait ScreenCaptureManager {
    type Buffer;
    type Issue;
    type Subscription: futures_core::Stream<Item = eros::Result<Rc<CapturedFrame<Self::Buffer, Self::Issue>>>>;

    fn subscribe(&mut self, screen_id: &ScreenId) -> eros::Result<Self::Subscription>;
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::kernel::screen_capture::CapturedFrame;

    #[derive(Debug)]
    struct NonCloneBuffer;

    #[test]
    fn one_captured_frame_can_be_shared_without_cloning_its_buffer() {
        let frame = Rc::new(CapturedFrame {
            buffer: NonCloneBuffer,
            issues: Vec::<()>::new(),
        });
        let other_subscriber = Rc::clone(&frame);

        assert!(Rc::ptr_eq(&frame, &other_subscriber));
    }
}
