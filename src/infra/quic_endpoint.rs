use std::net::SocketAddr;

use eros::Context;

pub(crate) struct QuicEndpoint {
    endpoint: compio::quic::Endpoint,
    client_config: compio::quic::ClientConfig,
}

impl QuicEndpoint {
    pub(crate) async fn new(bind_address: SocketAddr) -> eros::Result<Self> {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["rabbit".into()])
                .with_context(|| "Failed to generate temporary QUIC certificate")?;
        let certificate = cert.der().clone();
        let server = compio::quic::ServerBuilder::new_with_single_cert(
            vec![certificate],
            signing_key
                .serialize_der()
                .try_into()
                .expect("rcgen must serialize the generated private key as PKCS#8 DER"),
        )
        .with_context(|| "Failed to configure QUIC server certificate")?;
        let endpoint = server
            .bind(bind_address)
            .await
            .with_context(|| format!("Failed to bind QUIC endpoint to {bind_address}"))?;
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
}
