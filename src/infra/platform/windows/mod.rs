pub(crate) mod absolute_pointer;
pub(crate) mod client_video_probe;
pub(crate) mod frame_pipeline;
pub(crate) mod host_video_probe;
pub(crate) mod screen_capture;
pub(crate) mod screen_layout;
pub(crate) mod video_decoder;
pub(crate) mod video_encoder;
pub(crate) mod video_renderer;
pub(crate) mod worker_reaper;

use std::time::Duration;

pub(crate) use absolute_pointer::WindowsAbsolutePointerInjector;
pub(crate) use frame_pipeline::{WgcFramePipelineManager, WgcFramePipelineManagerState};
pub(crate) use screen_capture::{WgcScreenCaptureManager, WgcScreenCaptureManagerState};
pub(crate) use screen_layout::{
    WindowsScreenLayoutManager, WindowsScreenLayoutManagerState, create_screen_layout_manager_state,
};
pub(crate) use video_decoder::{WindowsDecodedFrame, WindowsVideoDecoder};
pub(crate) use video_encoder::WindowsVideoEncoder;
pub(crate) use video_renderer::{NativeVideoRenderer, NativeVideoViewport, OpenGlVideoRenderer};
pub(crate) use worker_reaper::{WorkerReaper, WorkerReaperHandle};

/// Creates the Windows Graphics Capture manager state.
pub(crate) fn create_screen_capture_manager_state(
    enable_probing: bool,
    probe_interval: Duration,
    _worker_reaper: crate::infra::WorkerReaperHandle,
) -> WgcScreenCaptureManagerState {
    WgcScreenCaptureManagerState::new(enable_probing, probe_interval)
}

/// Creates the Windows frame-pipeline manager state.
pub(crate) fn create_frame_pipeline_manager_state(
    _worker_reaper: crate::infra::WorkerReaperHandle,
) -> WgcFramePipelineManagerState {
    WgcFramePipelineManagerState::new()
}
