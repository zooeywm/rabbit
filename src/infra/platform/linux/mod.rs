mod client_video_probe;
mod dma_buf;
mod egl_dma_buf;
mod frame_pipeline;
mod gpu;
mod screen_capture;
mod screen_layout;
mod video_decoder;
mod video_encoder;
mod video_probe;
mod video_renderer;

use std::time::Duration;

pub(crate) use frame_pipeline::{GbmFramePipelineManager, GbmFramePipelineManagerState};
pub(crate) use screen_capture::{KmsScreenCaptureManager, KmsScreenCaptureManagerState};
pub(crate) use screen_layout::{
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_screen_layout_manager_state,
};
pub(crate) use video_decoder::{GStreamerDecodedFrame, GStreamerVideoDecoder};
pub(crate) use video_encoder::GStreamerVideoEncoder;
pub(crate) use video_renderer::{OpenGlVideoRenderer, WaylandVideoRenderer, WaylandVideoViewport};

/// Negotiates the Linux capture output requested by the selected encoder stack.
pub(crate) fn create_screen_capture_manager_state(
    enable_probing: bool,
    probe_interval: Duration,
    worker_reaper: crate::infra::WorkerReaperHandle,
) -> KmsScreenCaptureManagerState {
    KmsScreenCaptureManagerState::new(
        enable_probing,
        probe_interval,
        worker_reaper,
        video_encoder::va_vpp_input_profiles,
    )
}

/// Creates the frame-pipeline manager state selected for Linux.
pub(crate) fn create_frame_pipeline_manager_state(
    worker_reaper: crate::infra::WorkerReaperHandle,
) -> GbmFramePipelineManagerState {
    GbmFramePipelineManagerState::new(worker_reaper)
}
