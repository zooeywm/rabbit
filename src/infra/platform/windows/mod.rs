mod client_video_probe;
mod frame_pipeline;
mod host_video_probe;
mod input;
mod screen_capture;
mod screen_layout;
mod video_decoder;
mod video_encoder;
mod video_renderer;
mod worker_reaper;

use std::time::Duration;

pub(crate) use frame_pipeline::{WgcFramePipelineManager, WgcFramePipelineManagerState};
pub(crate) use input::WindowsRemoteInputInjector;
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
