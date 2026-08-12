use crate::app::container::root::outbound_port::{PacketizerManager, PacketizerManagerStateSpec};

use super::packetizer::FakePacketizerState;

#[derive(kudi::DepInj)]
#[target(FakePacketizerManagerImpl)]
pub(crate) struct FakePacketizerManagerState;

impl FakePacketizerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self)
    }
}

impl<Deps> PacketizerManager for FakePacketizerManagerImpl<Deps> {
    type State = FakePacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<Deps> {
        || Ok(FakePacketizerState::new())
    }
}
