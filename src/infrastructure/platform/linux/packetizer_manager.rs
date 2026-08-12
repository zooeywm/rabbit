use crate::app::container::root::outbound_port::{PacketizerManager, PacketizerManagerStateSpec};

#[derive(kudi::DepInj)]
#[target(LinuxPacketizerManagerImpl)]
pub(crate) struct LinuxPacketizerManagerState;

impl LinuxPacketizerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Linux packetization infrastructure has not been implemented");
    }
}

impl<Deps> PacketizerManager for LinuxPacketizerManagerImpl<Deps> {
    type State = LinuxPacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Linux packetization infrastructure has not been implemented")
    }
}
