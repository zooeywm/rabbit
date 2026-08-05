/// Controls the capacity of the frame pool owned by a capturer.
pub(crate) trait CaptureFramePoolCapacityController {
    /// Sets the number of frame resources maintained by the capturer.
    fn set_pool_size(&mut self, pool_size: usize) -> eros::Result<()>;
}
