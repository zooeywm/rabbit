pub(crate) struct ScreenCapturerContainer<State> {
    state: State,
}

impl<State> ScreenCapturerContainer<State> {
    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<State> From<State> for ScreenCapturerContainer<State> {
    fn from(state: State) -> Self {
        Self { state }
    }
}
