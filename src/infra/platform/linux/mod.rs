mod dma_buf;
mod frame_pipeline;
mod gpu;
mod screen_capture;
mod screen_layout;
mod video_encoder;
mod video_probe;

pub(crate) use frame_pipeline::{GbmFramePipelineManager, GbmFramePipelineManagerState};
pub(crate) use screen_capture::{
    KmsScreenCaptureManager, KmsScreenCaptureManagerState, create_screen_capture_manager_state,
};
pub(crate) use screen_layout::{
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_screen_layout_manager_state,
};
pub(crate) use video_encoder::GStreamerVideoEncoder;

/// Creates the frame-pipeline manager state selected for Linux.
pub(crate) fn create_frame_pipeline_manager_state(
    worker_reaper: crate::infra::WorkerReaperHandle,
) -> GbmFramePipelineManagerState {
    GbmFramePipelineManagerState::new(worker_reaper)
}
