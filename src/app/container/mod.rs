mod boilerplate;
pub(crate) mod capture_source;
pub(crate) mod root;
pub(crate) mod stream_pipeline;

pub(crate) use capture_source::CaptureSourceContainer;
pub(crate) use root::RootContainer;
pub(crate) use stream_pipeline::{LatestFrameSlot, StreamPipelineContainer};
