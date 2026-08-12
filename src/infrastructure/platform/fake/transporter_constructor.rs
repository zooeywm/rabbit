use std::cell::Cell;

use crate::{
    app::container::root::outbound_port::{
        TransporterConstructor, TransporterConstructorStateSpec,
    },
    infrastructure::platform::FakeTransporterState,
};

#[derive(kudi::DepInj)]
#[target(FakeTransporterConstructorImpl)]
pub(crate) struct FakeTransporterConstructorState {
    constructor_created: Cell<bool>,
}

impl FakeTransporterConstructorState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            constructor_created: Cell::new(false),
        })
    }
}

impl<Deps> TransporterConstructor for FakeTransporterConstructorImpl<Deps>
where
    Deps: AsRef<FakeTransporterConstructorState>,
{
    type State = FakeTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<Deps>,
    > {
        if self.prj_ref().as_ref().constructor_created.replace(true) {
            eros::bail!("Transporter constructor has already been created");
        }

        Ok(FakeTransporterState::new)
    }
}
