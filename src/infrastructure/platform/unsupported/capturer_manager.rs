use crate::{
    app::container::app_container::outbound_port::{CapturerManager, CapturerManagerStateSpec},
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

    fn compose_screen_capturer_state(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
