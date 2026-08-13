pub(crate) mod inbound;
pub(crate) mod outbound_port;

use std::marker::PhantomData;

use inbound::{ClientStreamControlReceiver, EncodedUnitReceiver, NetworkRequestReceiver};
use outbound_port::NetworkState;

use crate::app::runtime::AppMessage;

#[derive(Clone, Copy)]
pub(crate) struct NetworkMetricsHandle;

impl NetworkMetricsHandle {
    pub(crate) fn new() -> Self {
        Self
    }
}

pub(crate) struct NetworkContainer<State: NetworkState, Storage = State> {
    storage: Storage,
    encoded_unit_receiver: Option<EncodedUnitReceiver<State::EncodedBuffer>>,
    client_stream_control_receiver: Option<ClientStreamControlReceiver<State::ClientInput>>,
    network_request_receiver: Option<NetworkRequestReceiver>,
    app_message_sender: flume::Sender<AppMessage>,
    _state_type: PhantomData<fn() -> State>,
}

impl<State: NetworkState> NetworkContainer<State> {
    fn from_state(
        state: State,
        encoded_unit_receiver: EncodedUnitReceiver<State::EncodedBuffer>,
        client_stream_control_receiver: ClientStreamControlReceiver<State::ClientInput>,
        network_request_receiver: NetworkRequestReceiver,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> Self {
        Self {
            storage: state,
            encoded_unit_receiver: Some(encoded_unit_receiver),
            client_stream_control_receiver: Some(client_stream_control_receiver),
            network_request_receiver: Some(network_request_receiver),
            app_message_sender,
            _state_type: PhantomData,
        }
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.storage
    }

    pub(in crate::app::container::network) fn take_encoded_unit_receiver(
        &mut self,
    ) -> EncodedUnitReceiver<State::EncodedBuffer> {
        self.encoded_unit_receiver
            .take()
            .expect("Network encoded unit receiver should only be taken once")
    }

    pub(in crate::app::container::network) fn take_client_stream_control_receiver(
        &mut self,
    ) -> ClientStreamControlReceiver<State::ClientInput> {
        self.client_stream_control_receiver
            .take()
            .expect("Network Client stream control receiver should only be taken once")
    }

    pub(in crate::app::container::network) fn take_network_request_receiver(
        &mut self,
    ) -> NetworkRequestReceiver {
        self.network_request_receiver
            .take()
            .expect("Network request receiver should only be taken once")
    }

    pub(in crate::app::container::network) fn app_message_sender(
        &self,
    ) -> flume::Sender<AppMessage> {
        self.app_message_sender.clone()
    }
}

pub(crate) struct NetworkConfig<State: NetworkState> {
    config: State::Config,
}

pub(crate) type ConfiguredNetworkContainer<State> = NetworkContainer<State, NetworkConfig<State>>;

impl<State: NetworkState> ConfiguredNetworkContainer<State> {
    pub(crate) fn new(
        config: State::Config,
        encoded_unit_receiver: EncodedUnitReceiver<State::EncodedBuffer>,
        client_stream_control_receiver: ClientStreamControlReceiver<State::ClientInput>,
        network_request_receiver: NetworkRequestReceiver,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> Self {
        Self {
            storage: NetworkConfig { config },
            encoded_unit_receiver: Some(encoded_unit_receiver),
            client_stream_control_receiver: Some(client_stream_control_receiver),
            network_request_receiver: Some(network_request_receiver),
            app_message_sender,
            _state_type: PhantomData,
        }
    }

    pub(crate) fn initialize(self) -> eros::Result<NetworkContainer<State>> {
        Ok(NetworkContainer::from_state(
            State::new(self.storage.config)?,
            self.encoded_unit_receiver
                .expect("Configured Network container should contain encoded unit receiver"),
            self.client_stream_control_receiver.expect(
                "Configured Network container should contain Client stream control receiver",
            ),
            self.network_request_receiver
                .expect("Configured Network container should contain network request receiver"),
            self.app_message_sender,
        ))
    }
}
