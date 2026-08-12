mod inbound;

pub(crate) mod outbound_port;

use crate::app::container::{
    host::{HostContainer, outbound_port::CapturerManagerStateSpec},
    root::outbound_port::{TransporterConstructor, TransporterConstructorStateSpec},
};

pub(crate) type TransporterStateFor<NetworkConstructorState> =
    <NetworkConstructorState as TransporterConstructorStateSpec>::TransporterState;

pub(crate) struct AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    host: HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>,
    network_constructor_state: NetworkConstructorState,
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>
    AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    pub(crate) fn new(
        host: HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>,
        network_constructor_state: NetworkConstructorState,
    ) -> Self {
        Self {
            host,
            network_constructor_state,
        }
    }

    pub(crate) fn network_constructor_state(&self) -> &NetworkConstructorState {
        &self.network_constructor_state
    }

    pub(in crate::app) fn compose_transporter(
        &self,
    ) -> eros::Result<
        impl FnOnce() -> eros::Result<TransporterStateFor<NetworkConstructorState>>
        + Send
        + 'static
        + use<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>,
    >
    where
        NetworkConstructorState: TransporterConstructorStateSpec,
        Self: TransporterConstructor<State = NetworkConstructorState>,
    {
        TransporterConstructor::compose_transporter_state(self)
    }
}
