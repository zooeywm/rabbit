pub(crate) mod inbound;
pub(crate) mod outbound_port;

pub(crate) struct ScreenCaptureContainer<State> {
    state: State,
}

impl<State> ScreenCaptureContainer<State> {
    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<State> From<State> for ScreenCaptureContainer<State> {
    fn from(state: State) -> Self {
        Self { state }
    }
}
