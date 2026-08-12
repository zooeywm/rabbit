pub(crate) mod inbound;
pub(crate) mod outbound_port;

pub(crate) struct TransporterContainer<State> {
    state: State,
}

impl<State> TransporterContainer<State> {
    pub(crate) fn new(state: State) -> Self {
        Self { state }
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}
