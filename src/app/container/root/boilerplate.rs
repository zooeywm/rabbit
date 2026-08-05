use crate::{
    app::container::root::outbound_port::CaptureManager,
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{CaptureManagerImpl, CaptureManagerState, ScreenCapturerState},
};

use super::RootContainer;

impl AsRef<CaptureManagerState> for RootContainer {
    fn as_ref(&self) -> &CaptureManagerState {
        &self.capture_manager_state
    }
}

impl AsMut<CaptureManagerState> for RootContainer {
    fn as_mut(&mut self) -> &mut CaptureManagerState {
        &mut self.capture_manager_state
    }
}

impl CaptureManager for RootContainer {
    type ScreenCapturerState = ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        CaptureManagerImpl::inj_ref_mut(self).create_screen_capturer(capture_source_id)
    }
}
