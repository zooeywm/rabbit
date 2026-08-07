use std::convert::Infallible;

use crate::app::container::capture_source::outbound_port::{CaptureLoopAction, ScreenCapturer};

#[derive(kudi::DepInj)]
#[target(UnsupportedScreenCapturerImpl)]
pub(crate) struct UnsupportedScreenCapturerState {
    never: Infallible,
}

impl<Deps> ScreenCapturer for UnsupportedScreenCapturerImpl<Deps> {
    type CapturedFrame = Infallible;

    fn run<OnStarted, OnFrame>(
        &mut self,
        _initial_consumer_count: usize,
        _on_started: OnStarted,
        _on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
