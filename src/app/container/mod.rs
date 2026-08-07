pub(crate) mod app_container;
pub(crate) mod capture_source;
pub(crate) mod stream_pipeline;

pub(crate) use app_container::AppContainer;
pub(crate) use capture_source::{CaptureSourceContainer, CaptureWorker, ScreenCapturerContainer};
pub(crate) use stream_pipeline::{
    LatestFrameSlot, StreamPipelineContainer, StreamPipelineWorker, StreamPipelineWorkerHandle,
};
