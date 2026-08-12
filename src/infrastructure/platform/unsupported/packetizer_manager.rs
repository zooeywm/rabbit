use crate::app::container::root::outbound_port::{PacketizerManager, PacketizerManagerStateSpec};

#[derive(kudi::DepInj)]
#[target(UnsupportedPacketizerManagerImpl)]
pub(crate) struct UnsupportedPacketizerManagerState;

impl UnsupportedPacketizerManagerState {
    pub(crate) fn new() -> eros::Result<Self> {
        eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS,);
    }
}

impl<Deps> PacketizerManager for UnsupportedPacketizerManagerImpl<Deps> {
    type State = UnsupportedPacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<Deps> {
        || eros::bail!("Rabbit is unsupported on {}", std::env::consts::OS)
    }
}
