use crate::{
    app::container::root::outbound_port::{CapturerManager, CapturerManagerStateSpec},
    domain::stream::models::vo::CaptureSourceId,
};

use super::capturer::FakeScreenCapturerState;

#[derive(kudi::DepInj)]
#[target(FakeCapturerManagerImpl)]
pub(crate) struct FakeCapturerManagerState;

impl FakeCapturerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> CapturerManager for FakeCapturerManagerImpl<Deps> {
    type State = FakeCapturerManagerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState> {
        Ok(FakeScreenCapturerState::new(0))
    }
}
