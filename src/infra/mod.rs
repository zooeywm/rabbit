mod connection_endpoint;
mod connection_request;
#[cfg_attr(target_os = "linux", path = "platform/linux/mod.rs")]
#[cfg_attr(target_os = "windows", path = "platform/windows/mod.rs")]
mod platform;
mod quic_endpoint;
mod tcp_endpoint;
mod transport;
pub(crate) mod unsync_queue;

pub(crate) use connection_endpoint::{ConnectionEndpoint, IncomingConnection};
pub(crate) use connection_request::{
    DirectConnectionOutcome, PendingConnectionRequest, connect_transport, receive_request,
};
pub(crate) use platform::*;
pub(crate) use quic_endpoint::{QuicConnectOutcome, QuicEndpoint};
pub(crate) use tcp_endpoint::TcpEndpoint;
pub(crate) use transport::{SessionTransport, SessionTransportRecv, SessionTransportSend};
