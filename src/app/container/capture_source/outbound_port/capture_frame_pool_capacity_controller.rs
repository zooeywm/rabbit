pub(crate) trait CaptureFramePoolCapacityController {
    fn set_pool_size(&mut self, pool_size: usize) -> eros::Result<()>;
}
