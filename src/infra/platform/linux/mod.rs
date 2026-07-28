pub(crate) mod absolute_pointer;
pub(crate) mod client_video_probe;
pub(crate) mod dma_buf;
pub(crate) mod egl_dma_buf;
pub(crate) mod frame_pipeline;
pub(crate) mod gpu;
pub(crate) mod screen_capture;
pub(crate) mod screen_layout;
pub(crate) mod video_decoder;
pub(crate) mod video_encoder;
pub(crate) mod video_probe;
pub(crate) mod video_renderer;
pub(crate) mod worker_reaper;

use std::time::Duration;

pub(crate) use absolute_pointer::LinuxAbsolutePointerInjector;
pub(crate) use frame_pipeline::{
    GbmFramePipelineFrame, GbmFramePipelineManager, GbmFramePipelineManagerState,
};
pub(crate) use screen_capture::{KmsScreenCaptureManager, KmsScreenCaptureManagerState};
pub(crate) use screen_layout::{
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_screen_layout_manager_state,
};
pub(crate) use video_decoder::{GStreamerDecodedFrame, GStreamerVideoDecoder};
pub(crate) use video_encoder::{GStreamerVideoEncoder, record_frames_to_mp4};
pub(crate) use video_renderer::{
    OpenGlVideoRenderer, WaylandVideoRenderer as NativeVideoRenderer,
    WaylandVideoViewport as NativeVideoViewport,
};
pub(crate) use worker_reaper::{WorkerReaper, WorkerReaperHandle};

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
