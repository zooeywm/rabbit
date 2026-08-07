use std::{convert::Infallible, sync::Arc};

use crate::app::container::capture_source::outbound_port::{
    CaptureLoopAction, ScreenCapturer, ScreenCapturerControl,
};

#[derive(kudi::DepInj)]
#[target(LinuxScreenCapturerImpl)]
pub(crate) struct LinuxScreenCapturerState {
    never: Infallible,
}

impl<Deps> ScreenCapturer for LinuxScreenCapturerImpl<Deps> {
    type CapturedFrame = Infallible;

    fn control(&self) -> eros::Result<Arc<dyn ScreenCapturerControl>> {
        eros::bail!("Linux screen capturer infrastructure has not been implemented")
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
        eros::bail!("Linux screen capturer infrastructure has not been implemented");
    }
}
