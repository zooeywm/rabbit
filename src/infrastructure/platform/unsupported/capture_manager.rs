use std::convert::Infallible;

use crate::{
    app::container::root::outbound_port::CaptureManager,
    domain::stream::models::vo::CaptureSourceId,
};

pub(crate) type ScreenCapturerState = Infallible;

#[derive(kudi::DepInj)]
#[target(CaptureManagerImpl)]
pub(crate) struct CaptureManagerState {
    /// Prevents direct construction outside this module.
    _private: (),
}

impl CaptureManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> CaptureManager for CaptureManagerImpl<Deps> {
    type ScreenCapturerState = ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
