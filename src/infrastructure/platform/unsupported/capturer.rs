use std::{convert::Infallible, sync::Arc};

use crate::app::container::capture_source::outbound_port::{
    CaptureLoopAction, ScreenCapturer, ScreenCapturerControl,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedScreenCapturerImpl)]
pub(crate) struct UnsupportedScreenCapturerState {
    never: Infallible,
}

impl<Deps> ScreenCapturer for UnsupportedScreenCapturerImpl<Deps> {
    type CapturedFrame = Infallible;

    fn control(&self) -> eros::Result<Arc<dyn ScreenCapturerControl>> {
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
