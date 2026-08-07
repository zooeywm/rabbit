use crate::{
    app::container::app_container::outbound_port::{CapturerManager, CapturerManagerStateSpec},
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::FakeScreenCapturerState,
};

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

    fn screen_capturer_state_factory(
        &mut self,
        _capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<Deps> {
        || Ok(FakeScreenCapturerState::new())
    }
}
