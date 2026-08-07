use crate::{
    app::container::app_container::outbound_port::{CapturerManager, CapturerManagerStateSpec},
    domain::stream::models::vo::CaptureSourceId,
};

#[derive(kudi::DepInj)]
#[target(LinuxCapturerManagerImpl)]
pub(crate) struct LinuxCapturerManagerState;

impl LinuxCapturerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux screen capturer infrastructure has not been implemented");
    }
}

impl<Deps> CapturerManager for LinuxCapturerManagerImpl<Deps> {
    type State = LinuxCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Linux screen capturer infrastructure has not been implemented")
    }
}
