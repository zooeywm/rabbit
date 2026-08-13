pub(crate) trait EncoderFrameConverterState: Sized + 'static {
    fn new() -> eros::Result<Self>;
}
