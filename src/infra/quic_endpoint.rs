use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    ops::RangeInclusive,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use eros::Context;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tracing::{debug, info};

const BASE_PORT: u16 = 52731;
const LAST_PORT: u16 = BASE_PORT + 4;
const SERVER_NAME: &str = "rabbit";
const DATAGRAM_RECEIVE_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const UDP_SOCKET_BUFFER_SIZE: usize = 4 * 1024 * 1024;
pub(crate) const SELF_CONNECTION_CLOSE_REASON: &[u8] = b"Self-connection is not allowed";

pub(crate) enum QuicConnectOutcome {
    Connected(compio::quic::Connection),
    SelfConnection,
}

#[derive(Clone)]
pub(crate) struct QuicEndpoint {
    endpoint: compio::quic::Endpoint,
    client_config: compio::quic::ClientConfig,
    certificate: Bytes,
}

impl QuicEndpoint {
    pub(crate) async fn new() -> eros::Result<Self> {
        Self::new_with_bind_address(None).await
    }

    #[cfg(test)]
    pub(crate) async fn new_for_test() -> eros::Result<Self> {
        Self::new_with_bind_address(Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0))).await
    }

    async fn new_with_bind_address(bind_address: Option<SocketAddr>) -> eros::Result<Self> {
        let mut transport_config = compio::quic::TransportConfig::default();
        transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
        transport_config.datagram_receive_buffer_size(Some(DATAGRAM_RECEIVE_BUFFER_SIZE));
        let transport_config = Arc::new(transport_config);
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["rabbit".into()])
                .with_context(|| "Failed to generate temporary QUIC certificate")?;
        let certificate = cert.der().clone();
        let endpoint_certificate = Bytes::copy_from_slice(certificate.as_ref());
        let private_key = signing_key.serialize_der().try_into().map_err(|error| {
            eros::error!("Failed to decode generated PKCS#8 private key: {}", error)
        })?;
        let mut server_config =
            compio::quic::ServerBuilder::new_with_single_cert(vec![certificate], private_key)
                .with_context(|| "Failed to configure QUIC server certificate")?
                .build();
        server_config.transport_config(transport_config.clone());
        let endpoint = match bind_address {
            Some(bind_address) => create_endpoint(bind_address, server_config)
                .with_context(|| format!("Failed to bind test QUIC endpoint to {bind_address}"))?,
            None => bind_endpoint(server_config).await?,
        };
        let mut client_config =
            compio::quic::ClientBuilder::new_with_no_server_verification().build();
        client_config.transport_config(transport_config);

        Ok(Self {
            endpoint,
            client_config,
            certificate: endpoint_certificate,
        })
    }

    pub(crate) fn local_address(&self) -> eros::Result<SocketAddr> {
        Ok(self
            .endpoint
            .local_addr()
            .with_context(|| "Failed to read QUIC endpoint local address")?)
    }

    pub(crate) fn default_ports() -> RangeInclusive<u16> {
        BASE_PORT..=LAST_PORT
    }

    #[cfg(test)]
    pub(crate) async fn connect(
        &self,
        remote_address: SocketAddr,
    ) -> eros::Result<compio::quic::Connection> {
        match self.connect_outcome(remote_address).await? {
            QuicConnectOutcome::Connected(connection) => Ok(connection),
            QuicConnectOutcome::SelfConnection => {
                eros::bail!(
                    "Refusing to connect Rabbit to its own QUIC endpoint at {}",
                    remote_address
                )
            }
        }
    }

    pub(crate) async fn connect_outcome(
        &self,
        remote_address: SocketAddr,
    ) -> eros::Result<QuicConnectOutcome> {
        info!(
            event = "quic_connection_started",
            direction = "outgoing",
            %remote_address,
            "QUIC connection started"
        );
        let connecting = self
            .endpoint
            .connect(
                remote_address,
                SERVER_NAME,
                Some(self.client_config.clone()),
            )
            .with_context(|| format!("Failed to start QUIC connection to {remote_address}"))?;
        let started_at = Instant::now();
        let connection = connecting
            .await
            .with_context(|| format!("Failed to connect QUIC peer {remote_address}"))?;

        if peer_uses_certificate(&connection, &self.certificate) {
            connection.close(
                compio::quic::VarInt::from_u32(0),
                SELF_CONNECTION_CLOSE_REASON,
            );
            info!(
                event = "self_connection_rejected",
                %remote_address,
                "Self-connection rejected"
            );
            return Ok(QuicConnectOutcome::SelfConnection);
        }

        let elapsed = started_at.elapsed();
        let rtt = connection.rtt();

        info!(
            event = "quic_connection_established",
            direction = "outgoing",
            %remote_address,
            elapsed_ms = elapsed.as_millis(),
            rtt_ms = rtt.as_millis(),
            "QUIC connection established"
        );

        debug!(
            %remote_address,
            elapsed_ms = elapsed.as_millis(),
            rtt_ms = rtt.as_millis(),
            stats = ?connection.stats(),
            "Established outgoing QUIC connection"
        );

        Ok(QuicConnectOutcome::Connected(connection))
    }

    pub(crate) async fn accept_connection(&self) -> eros::Result<Option<compio::quic::Connection>> {
        let Some(incoming) = self.endpoint.wait_incoming().await else {
            return Ok(None);
        };
        let remote_address = incoming.remote_address();
        info!(
            event = "quic_connection_started",
            direction = "incoming",
            %remote_address,
            "QUIC connection started"
        );
        let started_at = Instant::now();
        let connection = incoming
            .await
            .with_context(|| format!("Failed to accept QUIC connection from {remote_address}"))?;
        let elapsed = started_at.elapsed();
        let rtt = connection.rtt();

        info!(
            event = "quic_connection_established",
            direction = "incoming",
            %remote_address,
            elapsed_ms = elapsed.as_millis(),
            rtt_ms = rtt.as_millis(),
            "QUIC connection established"
        );

        debug!(
            %remote_address,
            elapsed_ms = elapsed.as_millis(),
            rtt_ms = rtt.as_millis(),
            stats = ?connection.stats(),
            "Established incoming QUIC connection"
        );

        Ok(Some(connection))
    }
}

fn peer_uses_certificate(connection: &compio::quic::Connection, certificate: &[u8]) -> bool {
    connection.peer_identity().is_some_and(|peer_certificates| {
        peer_certificates
            .first()
            .is_some_and(|peer_certificate| peer_certificate.as_ref() == certificate)
    })
}

async fn bind_endpoint(
    server_config: compio::quic::ServerConfig,
) -> eros::Result<compio::quic::Endpoint> {
    for port in BASE_PORT..=LAST_PORT {
        let bind_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);

        match create_endpoint(bind_address, server_config.clone()) {
            Ok(endpoint) => return Ok(endpoint),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {}
            Err(error) => {
                return Ok(Err(error)
                    .with_context(|| format!("Failed to bind QUIC endpoint to {bind_address}"))?);
            }
        }
    }

    eros::bail!("QUIC ports {BASE_PORT} through {LAST_PORT} are already in use");
}

fn create_endpoint(
    bind_address: SocketAddr,
    server_config: compio::quic::ServerConfig,
) -> std::io::Result<compio::quic::Endpoint> {
    let domain = if bind_address.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    if bind_address.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_recv_buffer_size(UDP_SOCKET_BUFFER_SIZE)?;
    socket.set_send_buffer_size(UDP_SOCKET_BUFFER_SIZE)?;
    socket.bind(&SockAddr::from(bind_address))?;
    let receive_buffer_size = socket.recv_buffer_size()?;
    let send_buffer_size = socket.send_buffer_size()?;
    let socket = compio::net::UdpSocket::from_std(socket.into())?;
    let endpoint = compio::quic::Endpoint::new(
        socket,
        compio::quic::EndpointConfig::default(),
        Some(server_config),
        None,
    )?;

    info!(
        event = "quic_udp_socket_configured",
        %bind_address,
        receive_buffer_size,
        send_buffer_size,
        "Configured QUIC UDP socket buffers"
    );

    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddr};

    use crate::infra::{quic_endpoint::peer_uses_certificate, receive_request};

    #[test]
    fn rejects_only_the_endpoint_with_the_same_certificate() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let endpoint = crate::infra::QuicEndpoint::new_for_test()
                .await
                .expect("Test QUIC endpoint should start");
            let local_address = endpoint
                .local_address()
                .expect("Test QUIC endpoint address should be available");
            let remote_address = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), local_address.port());
            let accepting_endpoint = endpoint.clone();
            let accept_task = compio::runtime::spawn(async move {
                accepting_endpoint
                    .accept_connection()
                    .await
                    .expect("Self-connection should reach the accepting endpoint")
                    .expect("Self-connection should produce an incoming connection")
            });

            let result = endpoint.connect(remote_address).await;

            assert!(
                result.is_err(),
                "The endpoint must reject its own certificate"
            );
            let incoming = accept_task
                .await
                .expect("Self-connection accept task should finish");
            assert!(!peer_uses_certificate(&incoming, &endpoint.certificate));
            assert!(
                receive_request(incoming)
                    .await
                    .expect("Self-connection closure should not be a request error")
                    .is_none()
            );

            let other_endpoint = crate::infra::QuicEndpoint::new_for_test()
                .await
                .expect("Second test QUIC endpoint should start");
            let other_address = other_endpoint
                .local_address()
                .expect("Second test QUIC endpoint address should be available");
            let other_address = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), other_address.port());
            let accepting_endpoint = other_endpoint.clone();
            let accept_task = compio::runtime::spawn(async move {
                accepting_endpoint
                    .accept_connection()
                    .await
                    .expect("Connection should reach the second endpoint")
                    .expect("Second endpoint should produce an incoming connection")
            });

            let outgoing = endpoint
                .connect(other_address)
                .await
                .expect("A different local Rabbit endpoint should remain connectable");
            let _incoming = accept_task
                .await
                .expect("Second endpoint accept task should finish");

            assert!(!peer_uses_certificate(&outgoing, &endpoint.certificate));
        });
    }
}
