use eros::Result;

pub trait CaptureFramePoolCapacityController {
    fn set_pool_size(&mut self, pool_size: usize) -> Result<()>;
}
