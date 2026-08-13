pub(crate) trait NetworkState: Sized + 'static {
    type Config: Send + 'static;
    type EncodedBuffer;
    type ClientInput: Send + 'static;

    fn new(config: Self::Config) -> eros::Result<Self>;
}
