use crate::{
    app::container::root::outbound_port::CapturerManager,
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{CapturerManagerImpl, CapturerManagerState, ScreenCapturerState},
};

use super::RootContainer;

impl AsRef<CapturerManagerState> for RootContainer {
    fn as_ref(&self) -> &CapturerManagerState {
        &self.capturer_manager_state
    }
}

impl AsMut<CapturerManagerState> for RootContainer {
    fn as_mut(&mut self) -> &mut CapturerManagerState {
        &mut self.capturer_manager_state
    }
}

impl CapturerManager for RootContainer {
    type ScreenCapturerState = ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        CapturerManagerImpl::inj_ref_mut(self).create_screen_capturer(capture_source_id)
    }
}
