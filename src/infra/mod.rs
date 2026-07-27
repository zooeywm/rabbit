mod connection_endpoint;
mod connection_request;
#[cfg_attr(target_os = "linux", path = "platform/linux/mod.rs")]
#[cfg_attr(target_os = "windows", path = "platform/windows/mod.rs")]
mod platform;
mod quic_endpoint;
mod tcp_endpoint;
mod transport;
pub(crate) mod unsync_queue;
mod worker_reaper;

pub(crate) use connection_endpoint::{ConnectionEndpoint, IncomingConnection};
pub(crate) use connection_request::{
    DirectConnectionOutcome, PendingConnectionRequest, connect_transport, receive_request,
};
#[cfg(target_os = "linux")]
pub(crate) use platform::{
    GStreamerDecodedFrame, GStreamerVideoDecoder, GStreamerVideoEncoder, GbmFramePipelineFrame,
    GbmFramePipelineManager, GbmFramePipelineManagerState, KmsScreenCaptureManager,
    KmsScreenCaptureManagerState, NiriScreenLayoutManager, NiriScreenLayoutManagerState,
    record_frames_to_mp4,
};
pub(crate) use platform::{
    NativeVideoRenderer, NativeVideoViewport, OpenGlVideoRenderer,
    create_frame_pipeline_manager_state, create_screen_capture_manager_state,
    create_screen_layout_manager_state,
};
#[cfg(target_os = "windows")]
pub(crate) use platform::{
    WgcFramePipelineManager, WgcFramePipelineManagerState, WgcScreenCaptureManager,
    WgcScreenCaptureManagerState, WindowsDecodedFrame, WindowsScreenLayoutManager,
    WindowsScreenLayoutManagerState, WindowsVideoDecoder, WindowsVideoEncoder,
};
pub(crate) use quic_endpoint::{QuicConnectOutcome, QuicEndpoint};
pub(crate) use tcp_endpoint::TcpEndpoint;
pub(crate) use transport::{SessionTransport, SessionTransportRecv, SessionTransportSend};
pub(crate) use worker_reaper::{WorkerReaper, WorkerReaperHandle};
