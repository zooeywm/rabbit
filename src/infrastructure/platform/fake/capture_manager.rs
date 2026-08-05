use crate::{
    app::container::root::outbound_port::CaptureManager,
    domain::stream::models::vo::CaptureSourceId,
};

use super::capture::FakeScreenCapturerState;

#[derive(kudi::DepInj)]
#[target(FakeCaptureManagerImpl)]
pub(crate) struct FakeCaptureManagerState;

impl FakeCaptureManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> CaptureManager for FakeCaptureManagerImpl<Deps> {
    type ScreenCapturerState = FakeScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        Ok(FakeScreenCapturerState::new(0))
    }
}
