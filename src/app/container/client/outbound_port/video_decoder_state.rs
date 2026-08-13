pub(crate) trait VideoDecoderState: Sized + 'static {
    fn new() -> eros::Result<Self>;
}
