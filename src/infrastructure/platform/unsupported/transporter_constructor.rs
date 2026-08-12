use std::cell::Cell;

use crate::app::container::root::outbound_port::{
    TransporterConstructor, TransporterConstructorStateSpec,
};

#[derive(kudi::DepInj)]
#[target(UnsupportedTransporterConstructorImpl)]
pub(crate) struct UnsupportedTransporterConstructorState {
    constructor_created: Cell<bool>,
}

impl UnsupportedTransporterConstructorState {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            constructor_created: Cell::new(false),
        })
    }
}

impl<Deps> TransporterConstructor for UnsupportedTransporterConstructorImpl<Deps>
where
    Deps: AsRef<UnsupportedTransporterConstructorState>,
{
    type State = UnsupportedTransporterConstructorState;

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

        Ok(|| eros::bail!("Transporter is not implemented on this platform"))
    }
}
