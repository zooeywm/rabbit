use crate::{
    app::container::capture_source::outbound_port::{CaptureLoopAction, ScreenCapturer},
    infrastructure::support::media::{FrameLease, FramePool},
};

#[derive(Default)]
pub(crate) struct FakeCapturedFrame {
    pub(crate) capture_sequence: u64,
}

#[derive(kudi::DepInj)]
#[target(FakeScreenCapturerImpl)]
pub(crate) struct FakeScreenCapturerState {
    frame_pool: FramePool<FakeCapturedFrame>,

    next_capture_sequence: u64,
}

impl FakeScreenCapturerState {
    pub(crate) fn new() -> Self {
        Self {
            frame_pool: FramePool::new(0),
            next_capture_sequence: 0,
        }
    }
}

impl<Deps> ScreenCapturer for FakeScreenCapturerImpl<Deps>
where
    Deps: AsMut<FakeScreenCapturerState>,
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;

    fn run<OnStarted, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        mut on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        self.prj_ref_mut()
            .as_mut()
            .frame_pool
            .set_pool_size(frame_pool_size(initial_consumer_count));

        on_started()?;

        loop {
            let frame = {
                let state = self.prj_ref_mut().as_mut();
                let mut frame = state.frame_pool.blocking_acquire();
                frame.capture_sequence = state.next_capture_sequence;
                state.next_capture_sequence += 1;
                frame
            };

            match on_frame(frame)? {
                CaptureLoopAction::Continue { consumer_count } => {
                    self.prj_ref_mut()
                        .as_mut()
                        .frame_pool
                        .set_pool_size(frame_pool_size(consumer_count));
                }
                CaptureLoopAction::Stop => return Ok(()),
            }
        }
    }
}

fn frame_pool_size(consumer_count: usize) -> usize {
    consumer_count + 2
}
