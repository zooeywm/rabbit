use crate::{
    app::container::capture_source::outbound_port::{
        CaptureFramePoolCapacityController, ScreenCapturer,
    },
    infrastructure::support::media::{FrameLease, FramePool},
};

#[derive(Default)]
pub(crate) struct FakeCapturedFrame {
    /// The sequence assigned by the fake screen capturer.
    pub(crate) capture_sequence: u64,
}

#[derive(kudi::DepInj)]
#[target(FakeScreenCapturerImpl)]
pub(crate) struct FakeScreenCapturerState {
    /// The frame pool used by the fake screen capturer.
    pub(super) frame_pool: FramePool<FakeCapturedFrame>,

    /// The sequence to assign to the next captured frame.
    pub(super) next_capture_sequence: u64,
}

impl FakeScreenCapturerState {
    pub(crate) fn new(pool_size: usize) -> Self {
        Self {
            frame_pool: FramePool::new(pool_size),
            next_capture_sequence: 0,
        }
    }
}

impl<Deps> CaptureFramePoolCapacityController for FakeScreenCapturerImpl<Deps>
where
    Deps: AsMut<FakeScreenCapturerState>,
{
    fn set_pool_size(&mut self, pool_size: usize) -> eros::Result<()> {
        let state = self.prj_ref_mut().as_mut();
        state.frame_pool.set_pool_size(pool_size);
        Ok(())
    }
}

impl<Deps> ScreenCapturer for FakeScreenCapturerImpl<Deps>
where
    Deps: AsMut<FakeScreenCapturerState>,
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;

    fn capture_next(&mut self) -> eros::Result<Self::CapturedFrame> {
        let state = self.prj_ref_mut().as_mut();
        let mut frame = state.frame_pool.blocking_acquire();
        frame.capture_sequence = state.next_capture_sequence;
        state.next_capture_sequence += 1;
        Ok(frame)
    }
}
