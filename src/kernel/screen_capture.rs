use crate::kernel::screen_manager::ScreenId;

/// A composed physical-screen frame and any recoverable source-layer issues.
#[derive(Debug)]
pub struct CapturedFrame<Buffer, Issue> {
    pub buffer: Buffer,
    pub issues: Vec<Issue>,
}

pub struct ScreenCaptureSource<Lease, Receiver> {
    pub lease: Lease,
    pub receiver: Receiver,
}

/// Acquires one owned frame receiver and its local lifetime lease.
pub trait ScreenCaptureManager {
    type Lease;
    type Receiver: Send + 'static;

    fn acquire(
        &mut self,
        screen_id: &ScreenId,
    ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>>;
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::kernel::{
        screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
        screen_manager::ScreenId,
    };

    struct EmptyManager;

    impl ScreenCaptureManager for EmptyManager {
        type Lease = Rc<()>;
        type Receiver = ();

        fn acquire(
            &mut self,
            _screen_id: &ScreenId,
        ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>> {
            Ok(ScreenCaptureSource {
                lease: Rc::new(()),
                receiver: (),
            })
        }
    }

    #[test]
    fn capture_lease_can_remain_local_while_receiver_is_sendable() {
        let ScreenCaptureSource { lease, receiver } = EmptyManager
            .acquire(&ScreenId(1))
            .expect("Screen capture source should be acquired");

        assert_eq!(Rc::strong_count(&lease), 1);
        assert_eq!(receiver, ());
    }
}
