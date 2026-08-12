pub(crate) mod inbound;
pub(crate) mod outbound_port;

#[derive(Clone, Copy)]
pub(crate) struct NetworkMetricsHandle;

impl NetworkMetricsHandle {
    pub(crate) fn new() -> Self {
        Self
    }
}

pub(crate) struct NetworkContainer<State> {
    state: State,
}

impl<State> NetworkContainer<State> {
    pub(crate) fn new(state: State) -> Self {
        Self { state }
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}
