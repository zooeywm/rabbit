use crate::{
    app::container::root::outbound_port::{CapturerManager, CapturerManagerStateSpec},
    domain::stream::models::vo::CaptureSourceId,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedCapturerManagerImpl)]
pub(crate) struct UnsupportedCapturerManagerState;

impl UnsupportedCapturerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> CapturerManager for UnsupportedCapturerManagerImpl<Deps> {
    type State = UnsupportedCapturerManagerState;

    fn create_screen_capturer(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}
