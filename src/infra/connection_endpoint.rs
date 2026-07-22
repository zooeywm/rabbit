use std::net::SocketAddr;

use crate::{
    app::config::NetworkTransport,
    infra::{QuicEndpoint, TcpEndpoint},
};

#[derive(Clone)]
pub(crate) enum ConnectionEndpoint {
    Quic(QuicEndpoint),
    Tcp(TcpEndpoint),
}

pub(crate) enum IncomingConnection {
    Quic(compio::quic::Connection),
    Tcp {
        stream: compio::net::TcpStream,
        remote_address: SocketAddr,
        endpoint_identity: [u8; 16],
    },
}

impl ConnectionEndpoint {
    pub(crate) async fn new(transport: NetworkTransport) -> eros::Result<Self> {
        match transport {
            NetworkTransport::Quic => Ok(Self::Quic(QuicEndpoint::new().await?)),
            NetworkTransport::Tcp => Ok(Self::Tcp(TcpEndpoint::new().await?)),
        }
    }

    pub(crate) fn local_address(&self) -> eros::Result<SocketAddr> {
        match self {
            Self::Quic(endpoint) => endpoint.local_address(),
            Self::Tcp(endpoint) => endpoint.local_address(),
        }
    }

    pub(crate) async fn accept_connection(&self) -> eros::Result<Option<IncomingConnection>> {
        match self {
            Self::Quic(endpoint) => Ok(endpoint
                .accept_connection()
                .await?
                .map(IncomingConnection::Quic)),
            Self::Tcp(endpoint) => {
                let (stream, remote_address) = endpoint.accept_connection().await?;
                Ok(Some(IncomingConnection::Tcp {
                    stream,
                    remote_address,
                    endpoint_identity: endpoint.identity(),
                }))
            }
        }
    }
}

// Focused test: cargo test infra::connection_request::tests::tcp_connection_request_establishes_transport_after_approval --lib
