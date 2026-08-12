use std::time::Instant;

use eros::Context;

use crate::{
    app::container::{
        root::outbound_port::MetricsRecorder,
        screen_capture::outbound_port::{CaptureLoopAction, ScreenCapturer, ScreenCapturerControl},
    },
    domain::stream::models::vo::{CaptureSourceId, FrameId},
    infrastructure::support::media::{FrameLease, FramePool, FramePoolWaker},
};

pub(crate) struct FakeCapturedFrame {
    pub(crate) frame_id: FrameId,
}

impl Default for FakeCapturedFrame {
    fn default() -> Self {
        Self {
            frame_id: FrameId::new(CaptureSourceId::new(0), 0),
        }
    }
}

#[derive(kudi::DepInj)]
#[target(FakeScreenCapturerImpl)]
pub(crate) struct FakeScreenCapturerState {
    frame_pool: FramePool<FakeCapturedFrame>,

    capture_source_id: CaptureSourceId,

    next_frame_id: u64,

    control_sender: flume::Sender<()>,

    control_receiver: flume::Receiver<()>,
}

impl FakeScreenCapturerState {
    pub(crate) fn new(capture_source_id: CaptureSourceId) -> Self {
        let (control_sender, control_receiver) = flume::unbounded();

        Self {
            frame_pool: FramePool::new(0),
            capture_source_id,
            next_frame_id: 0,
            control_sender,
            control_receiver,
        }
    }
}

pub(crate) struct FakeScreenCapturerControl {
    control_sender: flume::Sender<()>,
    frame_pool_waker: FramePoolWaker<FakeCapturedFrame>,
}

impl ScreenCapturerControl for FakeScreenCapturerControl {
    fn wake(&self) -> eros::Result<()> {
        self.control_sender
            .send(())
            .with_context(|| "Fake screen capturer stopped before control wakeup")?;
        self.frame_pool_waker.wake();
        Ok(())
    }
}

impl<Deps> ScreenCapturer for FakeScreenCapturerImpl<Deps>
where
    Deps: AsRef<FakeScreenCapturerState> + AsMut<FakeScreenCapturerState> + MetricsRecorder,
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;
    type Control = FakeScreenCapturerControl;

    fn control(&self) -> eros::Result<Self::Control> {
        Ok(FakeScreenCapturerControl {
            control_sender: self.prj_ref().as_ref().control_sender.clone(),
            frame_pool_waker: self.prj_ref().as_ref().frame_pool.waker(),
        })
    }

    fn run<OnStarted, OnControl, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        mut on_control: OnControl,
        mut on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        self.prj_ref_mut()
            .as_mut()
            .frame_pool
            .set_pool_size(frame_pool_size(initial_consumer_count));

        on_started()?;

        loop {
            while self.prj_ref().as_ref().control_receiver.try_recv().is_ok() {
                match on_control()? {
                    CaptureLoopAction::Continue { consumer_count } => {
                        self.prj_ref_mut()
                            .as_mut()
                            .frame_pool
                            .set_pool_size(frame_pool_size(consumer_count));
                    }
                    CaptureLoopAction::Stop => return Ok(()),
                }
            }

            let capture_started_at = Instant::now();
            let frame = {
                let state = self.prj_ref_mut().as_mut();
                let Some(mut frame) = state.frame_pool.blocking_acquire_interruptibly() else {
                    continue;
                };
                frame.frame_id = FrameId::new(state.capture_source_id, state.next_frame_id);
                state.next_frame_id = state
                    .next_frame_id
                    .checked_add(1)
                    .with_context(|| "Fake capture frame ID space is exhausted")?;
                frame
            };

            self.prj_ref()
                .record_captured_frame(frame.frame_id, capture_started_at.elapsed());

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
    #[cfg(feature = "test-ui")]
    if consumer_count == 0 {
        return 1;
    }

    consumer_count + 2
}
