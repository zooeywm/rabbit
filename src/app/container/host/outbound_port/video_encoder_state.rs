pub(crate) trait VideoEncoderState: Sized + 'static {
    fn new() -> eros::Result<Self>;
}
