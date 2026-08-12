pub(crate) trait VideoDecodeUnit {
    fn is_recovery_point(&self) -> bool;
}

impl VideoDecodeUnit for std::convert::Infallible {
    fn is_recovery_point(&self) -> bool {
        match *self {}
    }
}
