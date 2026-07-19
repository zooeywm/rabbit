mod connection_request;
#[cfg_attr(target_os = "linux", path = "platform/linux/mod.rs")]
#[cfg_attr(target_os = "windows", path = "platform/windows/mod.rs")]
mod platform;
mod quic_endpoint;
mod rayon_thread_pool;
mod transport;
pub(crate) mod unsync_queue;

pub(crate) use connection_request::{
    PendingQuicConnectionRequest, connect_transport, receive_request,
};
pub(crate) use platform::{
    GStreamerVideoEncoder, GbmFramePipelineFrame, GbmFramePipelineManager,
    GbmFramePipelineManagerState, KmsScreenCaptureManager, KmsScreenCaptureManagerState,
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_frame_pipeline_manager_state,
    create_screen_capture_manager_state, create_screen_layout_manager_state,
};
pub(crate) use quic_endpoint::QuicEndpoint;
pub(crate) use rayon_thread_pool::{RayonThreadPool, RayonThreadPoolState};
pub(crate) use transport::{QuicTransport, QuicTransportRecv, QuicTransportSend};
