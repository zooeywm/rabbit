mod inbound;

pub(crate) mod outbound_port;

use crate::app::container::root::outbound_port::{
    TransporterConstructor, TransporterConstructorStateSpec,
};

pub(crate) type TransporterStateFor<NetworkConstructorState> =
    <NetworkConstructorState as TransporterConstructorStateSpec>::TransporterState;

pub(crate) struct AppContainer<Host, Client, NetworkConstructorState> {
    host: Host,
    client: Client,
    network_constructor_state: NetworkConstructorState,
}

impl<Host, Client, NetworkConstructorState> AppContainer<Host, Client, NetworkConstructorState> {
    pub(crate) fn new(
        host: Host,
        client: Client,
        network_constructor_state: NetworkConstructorState,
    ) -> Self {
        Self {
            host,
            client,
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
        + use<Host, Client, NetworkConstructorState>,
    >
    where
        NetworkConstructorState: TransporterConstructorStateSpec,
        Self: TransporterConstructor<State = NetworkConstructorState>,
    {
        TransporterConstructor::compose_transporter_state(self)
    }
}
