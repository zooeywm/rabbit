pub(crate) trait TransporterConstructorStateSpec {
    type TransporterState: 'static;
}

pub(crate) trait TransporterConstructor {
    type State: TransporterConstructorStateSpec;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<Self>,
    >;
}
