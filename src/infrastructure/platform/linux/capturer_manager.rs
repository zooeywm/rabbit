use crate::{
    app::container::root::outbound_port::CapturerManager,
    domain::stream::models::vo::CaptureSourceId,
};

use super::capturer::LinuxScreenCapturerState;

#[derive(kudi::DepInj)]
#[target(LinuxCapturerManagerImpl)]
pub(crate) struct LinuxCapturerManagerState;

impl LinuxCapturerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux screen capturer infrastructure has not been implemented");
    }
}

impl<Deps> CapturerManager for LinuxCapturerManagerImpl<Deps> {
    type ScreenCapturerState = LinuxScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        eros::bail!("Linux screen capturer infrastructure has not been implemented");
    }
}
