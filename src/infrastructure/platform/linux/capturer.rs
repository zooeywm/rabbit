use std::convert::Infallible;

use crate::{
    app::container::{
        host::outbound_port::ScreenCapturerState,
        screen_capture::outbound_port::{CaptureLoopAction, ScreenCapturer},
    },
    domain::stream::models::vo::CaptureSourceId,
};

#[derive(kudi::DepInj)]
#[target(LinuxScreenCapturerImpl)]
pub(crate) struct LinuxScreenCapturerState {
    never: Infallible,
}

impl ScreenCapturerState for LinuxScreenCapturerState {
    fn new(_capture_source_id: CaptureSourceId) -> eros::Result<Self> {
        eros::bail!("Linux screen capturer infrastructure has not been implemented")
    }
}

impl<Deps> ScreenCapturer for LinuxScreenCapturerImpl<Deps> {
    type CapturedFrame = Infallible;
    type Control = Infallible;

    fn control(&self) -> eros::Result<Self::Control> {
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
