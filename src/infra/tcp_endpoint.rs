use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    ops::RangeInclusive,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use eros::Context;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tracing::{debug, info};

const BASE_PORT: u16 = 52731;
const LAST_PORT: u16 = BASE_PORT + 4;
static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub(crate) struct TcpEndpoint {
    listener: compio::net::TcpListener,
    identity: [u8; 16],
}

impl TcpEndpoint {
    pub(crate) async fn new() -> eros::Result<Self> {
        Self::new_with_bind_address(None).await
    }

    #[cfg(test)]
    pub(crate) async fn new_for_test() -> eros::Result<Self> {
        Self::new_with_bind_address(Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))).await
    }

    async fn new_with_bind_address(bind_address: Option<SocketAddr>) -> eros::Result<Self> {
        let listener = match bind_address {
            Some(bind_address) => create_listener(bind_address)
                .with_context(|| format!("Failed to bind test TCP endpoint to {bind_address}"))?,
            None => bind_listener().await?,
        };

        Ok(Self {
            listener,
            identity: endpoint_identity(),
        })
    }

    pub(crate) fn local_address(&self) -> eros::Result<SocketAddr> {
        Ok(self
            .listener
            .local_addr()
            .with_context(|| "Failed to read TCP endpoint local address")?)
    }

    pub(crate) fn default_ports() -> RangeInclusive<u16> {
        BASE_PORT..=LAST_PORT
    }

    pub(crate) fn identity(&self) -> [u8; 16] {
        self.identity
    }

    pub(crate) async fn connect(
        &self,
        remote_address: SocketAddr,
    ) -> eros::Result<compio::net::TcpStream> {
        info!(
            event = "tcp_connection_started",
            direction = "outgoing",
            %remote_address,
            "TCP connection started"
        );
        let started_at = std::time::Instant::now();
        let stream = compio::net::TcpStream::connect(remote_address)
            .await
            .with_context(|| format!("Failed to connect TCP peer {remote_address}"))?;
        stream
            .set_nodelay(true)
            .with_context(|| format!("Failed to enable TCP_NODELAY for {remote_address}"))?;

        info!(
            event = "tcp_connection_established",
            direction = "outgoing",
            %remote_address,
            elapsed_ms = started_at.elapsed().as_millis(),
            "TCP connection established"
        );

        Ok(stream)
    }

    pub(crate) async fn accept_connection(
        &self,
    ) -> eros::Result<(compio::net::TcpStream, SocketAddr)> {
        let (stream, remote_address) = self
            .listener
            .accept()
            .await
            .with_context(|| "Failed to accept TCP connection")?;
        stream
            .set_nodelay(true)
            .with_context(|| format!("Failed to enable TCP_NODELAY for {remote_address}"))?;

        info!(
            event = "tcp_connection_established",
            direction = "incoming",
            %remote_address,
            "TCP connection established"
        );
        debug!(%remote_address, "Accepted incoming TCP connection");

        Ok((stream, remote_address))
    }
}

async fn bind_listener() -> eros::Result<compio::net::TcpListener> {
    for port in BASE_PORT..=LAST_PORT {
        let bind_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);

        match create_listener(bind_address) {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {}
            Err(error) => {
                return Ok(Err(error)
                    .with_context(|| format!("Failed to bind TCP endpoint to {bind_address}"))?);
            }
        }
    }

    eros::bail!(
        "TCP ports {} through {} are already in use",
        BASE_PORT,
        LAST_PORT
    );
}

fn create_listener(bind_address: SocketAddr) -> std::io::Result<compio::net::TcpListener> {
    let domain = if bind_address.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if bind_address.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_reuse_address(true)?;
    socket.bind(&SockAddr::from(bind_address))?;
    socket.listen(128)?;

    compio::net::TcpListener::from_std(socket.into())
}

fn endpoint_identity() -> [u8; 16] {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = u128::from(ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(std::process::id()) << 96;

    (elapsed ^ sequence ^ process).to_be_bytes()
}

// Focused test: cargo test infra::connection_request::tests::tcp_endpoint_rejects_its_own_connection_before_approval --lib
