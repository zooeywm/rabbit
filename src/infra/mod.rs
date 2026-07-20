mod connection_request;
#[cfg_attr(target_os = "linux", path = "platform/linux/mod.rs")]
#[cfg_attr(target_os = "windows", path = "platform/windows/mod.rs")]
mod platform;
mod quic_endpoint;
mod transport;
pub(crate) mod unsync_queue;
mod worker_reaper;

pub(crate) use connection_request::{
    DirectConnectionOutcome, PendingQuicConnectionRequest, connect_transport, receive_request,
};
pub(crate) use platform::{
    GStreamerDecodedFrame, GStreamerVideoDecoder, GStreamerVideoEncoder, GbmFramePipelineManager,
    GbmFramePipelineManagerState, KmsScreenCaptureManager, KmsScreenCaptureManagerState,
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, OpenGlVideoRenderer,
    create_frame_pipeline_manager_state, create_screen_capture_manager_state,
    create_screen_layout_manager_state,
};
pub(crate) use quic_endpoint::{QuicConnectOutcome, QuicEndpoint};
pub(crate) use transport::{QuicTransport, QuicTransportRecv, QuicTransportSend};
pub(crate) use worker_reaper::{WorkerReaper, WorkerReaperHandle};
