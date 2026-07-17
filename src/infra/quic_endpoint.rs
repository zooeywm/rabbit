use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    ops::RangeInclusive,
    sync::Arc,
    time::{Duration, Instant},
};

use eros::Context;
use tracing::{debug, info};

const BASE_PORT: u16 = 52731;
const LAST_PORT: u16 = BASE_PORT + 4;
const SERVER_NAME: &str = "rabbit";

#[derive(Clone)]
pub(crate) struct QuicEndpoint {
    endpoint: compio::quic::Endpoint,
    client_config: compio::quic::ClientConfig,
}

impl QuicEndpoint {
    pub(crate) async fn new() -> eros::Result<Self> {
        let mut transport_config = compio::quic::TransportConfig::default();
        transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
        let transport_config = Arc::new(transport_config);
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["rabbit".into()])
                .with_context(|| "Failed to generate temporary QUIC certificate")?;
        let certificate = cert.der().clone();
        let mut server_config = compio::quic::ServerBuilder::new_with_single_cert(
            vec![certificate],
            signing_key
                .serialize_der()
                .try_into()
                .expect("rcgen must serialize the generated private key as PKCS#8 DER"),
        )
        .with_context(|| "Failed to configure QUIC server certificate")?
        .build();
        server_config.transport_config(transport_config.clone());
        let endpoint = bind_endpoint(server_config).await?;
        let mut client_config =
            compio::quic::ClientBuilder::new_with_no_server_verification().build();
        client_config.transport_config(transport_config);

        Ok(Self {
            endpoint,
            client_config,
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

    pub(crate) async fn connect(
        &self,
        remote_address: SocketAddr,
    ) -> eros::Result<compio::quic::Connection> {
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
            .with_context(|| {
                format!("Failed to start QUIC connection to {remote_address}")
            })?;
        let started_at = Instant::now();
        let connection = connecting
            .await
            .with_context(|| format!("Failed to connect QUIC peer {remote_address}"))?;
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

        Ok(connection)
    }

    pub(crate) async fn accept_connection(
        &self,
    ) -> eros::Result<Option<compio::quic::Connection>> {
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
        let connection = incoming.await.with_context(|| {
            format!("Failed to accept QUIC connection from {remote_address}")
        })?;
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

async fn bind_endpoint(
    server_config: compio::quic::ServerConfig,
) -> eros::Result<compio::quic::Endpoint> {
    for port in BASE_PORT..=LAST_PORT {
        let bind_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);

        match compio::quic::Endpoint::server(bind_address, server_config.clone()).await {
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
