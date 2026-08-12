pub(crate) trait PacketizerManagerStateSpec {
    type PacketizerState: 'static;
}

pub(crate) trait PacketizerManager {
    type State: PacketizerManagerStateSpec;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<Self>;
}
