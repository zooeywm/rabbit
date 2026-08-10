use std::convert::Infallible;

use crate::app::container::screen_capture::outbound_port::{CaptureLoopAction, ScreenCapturer};

#[derive(kudi::DepInj)]
#[target(UnsupportedScreenCapturerImpl)]
pub(crate) struct UnsupportedScreenCapturerState {
    never: Infallible,
}

impl<Deps> ScreenCapturer for UnsupportedScreenCapturerImpl<Deps> {
    type CapturedFrame = Infallible;
    type Control = Infallible;

    fn control(&self) -> eros::Result<Self::Control> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }

    fn run<OnStarted, OnControl, OnFrame>(
        &mut self,
        _initial_consumer_count: usize,
        _on_started: OnStarted,
        _on_control: OnControl,
        _on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
