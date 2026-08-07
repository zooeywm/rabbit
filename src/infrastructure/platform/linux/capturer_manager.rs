use crate::{
    app::container::root::outbound_port::{CapturerManager, CapturerManagerStateSpec},
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

    fn screen_capturer_state_factory(
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
