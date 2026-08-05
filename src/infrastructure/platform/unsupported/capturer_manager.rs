use std::convert::Infallible;

use crate::{
    app::container::root::outbound_port::CapturerManager,
    domain::stream::models::vo::CaptureSourceId,
};

pub(crate) type ScreenCapturerState = Infallible;

#[derive(kudi::DepInj)]
#[target(CapturerManagerImpl)]
pub(crate) struct CapturerManagerState {
    /// Prevents direct construction outside this module.
    _private: (),
}

impl CapturerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> CapturerManager for CapturerManagerImpl<Deps> {
    type ScreenCapturerState = ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
