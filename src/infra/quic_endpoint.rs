use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv6Addr, SocketAddr},
};

use eros::Context;

const BASE_PORT: u16 = 52731;
const LAST_PORT: u16 = BASE_PORT + 4;
const SERVER_NAME: &str = "rabbit";

pub(crate) struct QuicEndpoint {
    endpoint: compio::quic::Endpoint,
    client_config: compio::quic::ClientConfig,
}

impl QuicEndpoint {
    pub(crate) async fn new() -> eros::Result<Self> {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["rabbit".into()])
                .with_context(|| "Failed to generate temporary QUIC certificate")?;
        let certificate = cert.der().clone();
        let server_config = compio::quic::ServerBuilder::new_with_single_cert(
            vec![certificate],
            signing_key
                .serialize_der()
                .try_into()
                .expect("rcgen must serialize the generated private key as PKCS#8 DER"),
        )
        .with_context(|| "Failed to configure QUIC server certificate")?
        .build();
        let endpoint = bind_endpoint(server_config).await?;
        let client_config =
            compio::quic::ClientBuilder::new_with_no_server_verification().build();

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

    pub(crate) async fn connect(
        &self,
        remote_address: SocketAddr,
    ) -> eros::Result<compio::quic::Connection> {
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

        Ok(connecting
            .await
            .with_context(|| format!("Failed to connect QUIC peer {remote_address}"))?)
    }

    pub(crate) async fn accept_connection(
        &self,
    ) -> eros::Result<Option<compio::quic::Connection>> {
        let Some(incoming) = self.endpoint.wait_incoming().await else {
            return Ok(None);
        };
        let remote_address = incoming.remote_address();

        Ok(Some(incoming.await.with_context(|| {
            format!("Failed to accept QUIC connection from {remote_address}")
        })?))
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
