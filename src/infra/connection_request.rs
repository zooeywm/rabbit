use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use bytes::{BufMut, Bytes, BytesMut};
use compio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use eros::Context;
use tracing::{debug, info};

use crate::{
    infra::{
        ConnectionEndpoint, IncomingConnection, QuicConnectOutcome, QuicEndpoint, SessionTransport,
        TcpEndpoint,
        transport::{QuicTransport, TcpTransport},
    },
    kernel::connection_request::{ConnectionRequest, ConnectionResponse},
};

const REQUESTER_NAME_LENGTH_SIZE: usize = size_of::<u16>();
const RESPONSE_SIZE: usize = size_of::<u8>();
const TCP_REQUEST_MAGIC: &[u8; 5] = b"RBTC\x01";
const TCP_ENDPOINT_IDENTITY_SIZE: usize = 16;

pub(crate) enum DirectConnectionOutcome {
    Connected(SessionTransport),
    Rejected,
    SelfConnection,
}

pub(crate) enum PendingConnectionRequest {
    Quic(PendingQuicConnectionRequest),
    Tcp(PendingTcpConnectionRequest),
}

pub(crate) struct PendingQuicConnectionRequest {
    request: ConnectionRequest,
    remote_address: SocketAddr,
    connection: compio::quic::Connection,
    response_stream: compio::quic::SendStream,
}

pub(crate) struct PendingTcpConnectionRequest {
    request: ConnectionRequest,
    remote_address: SocketAddr,
    stream: compio::net::TcpStream,
}

impl From<compio::quic::Connection> for IncomingConnection {
    fn from(connection: compio::quic::Connection) -> Self {
        Self::Quic(connection)
    }
}

pub(crate) async fn connect_transport(
    endpoint: &ConnectionEndpoint,
    remote_ip: IpAddr,
    remote_port: Option<u16>,
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    match endpoint {
        ConnectionEndpoint::Quic(endpoint) => {
            connect_quic_transport(endpoint, remote_ip, remote_port, request).await
        }
        ConnectionEndpoint::Tcp(endpoint) => {
            connect_tcp_transport(endpoint, remote_ip, remote_port, request).await
        }
    }
}

async fn connect_quic_transport(
    endpoint: &QuicEndpoint,
    remote_ip: IpAddr,
    remote_port: Option<u16>,
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    if let Some(remote_port) = remote_port {
        let remote_address = SocketAddr::new(remote_ip, remote_port);
        let connection = match endpoint.connect_outcome(remote_address).await? {
            QuicConnectOutcome::Connected(connection) => connection,
            QuicConnectOutcome::SelfConnection => {
                return Ok(DirectConnectionOutcome::SelfConnection);
            }
        };

        return request_quic_transport(connection, request).await;
    }

    let mut last_error = None;

    for remote_port in QuicEndpoint::default_ports() {
        let remote_address = SocketAddr::new(remote_ip, remote_port);

        match endpoint.connect_outcome(remote_address).await {
            Ok(QuicConnectOutcome::Connected(connection)) => {
                return request_quic_transport(connection, request).await;
            }
            Ok(QuicConnectOutcome::SelfConnection) => {
                return Ok(DirectConnectionOutcome::SelfConnection);
            }
            Err(error) => last_error = Some(error),
        }
    }

    let Some(last_error) = last_error else {
        eros::bail!("Rabbit default QUIC port range is empty");
    };

    Err(last_error).with_context(|| {
        format!("Failed to connect Rabbit at any default QUIC port on {remote_ip}")
    })
}

async fn connect_tcp_transport(
    endpoint: &TcpEndpoint,
    remote_ip: IpAddr,
    remote_port: Option<u16>,
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    if let Some(remote_port) = remote_port {
        let remote_address = SocketAddr::new(remote_ip, remote_port);
        let stream = endpoint.connect(remote_address).await?;

        return request_tcp_transport(stream, endpoint.identity(), request).await;
    }

    let mut last_error = None;

    for remote_port in TcpEndpoint::default_ports() {
        let remote_address = SocketAddr::new(remote_ip, remote_port);

        match endpoint.connect(remote_address).await {
            Ok(stream) => {
                return request_tcp_transport(stream, endpoint.identity(), request).await;
            }
            Err(error) => last_error = Some(error),
        }
    }

    let Some(last_error) = last_error else {
        eros::bail!("Rabbit default TCP port range is empty");
    };

    Err(last_error)
        .with_context(|| format!("Failed to connect Rabbit at any default TCP port on {remote_ip}"))
}

async fn request_quic_transport(
    connection: compio::quic::Connection,
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    let remote_address = connection.remote_address();
    let (mut request_stream, mut response_stream) = connection
        .open_bi_wait()
        .await
        .with_context(|| "Failed to open QUIC connection request stream")?;

    send_quic_request(&mut request_stream, request).await?;

    let response = recv_quic_response(&mut response_stream).await?;
    log_response(remote_address, response);

    match response {
        ConnectionResponse::Accepted => Ok(DirectConnectionOutcome::Connected(
            SessionTransport::Quic(QuicTransport::open(connection).await?),
        )),
        ConnectionResponse::Rejected => Ok(DirectConnectionOutcome::Rejected),
        ConnectionResponse::SelfConnection => Ok(DirectConnectionOutcome::SelfConnection),
    }
}

async fn request_tcp_transport(
    mut stream: compio::net::TcpStream,
    endpoint_identity: [u8; TCP_ENDPOINT_IDENTITY_SIZE],
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    let remote_address = stream
        .peer_addr()
        .with_context(|| "Failed to read TCP connection request peer address")?;
    send_tcp_request(&mut stream, endpoint_identity, request).await?;
    let response = recv_tcp_response(&mut stream).await?;
    log_response(remote_address, response);

    match response {
        ConnectionResponse::Accepted => Ok(DirectConnectionOutcome::Connected(
            SessionTransport::Tcp(TcpTransport::new(stream)?),
        )),
        ConnectionResponse::Rejected => Ok(DirectConnectionOutcome::Rejected),
        ConnectionResponse::SelfConnection => Ok(DirectConnectionOutcome::SelfConnection),
    }
}

pub(crate) async fn receive_request(
    connection: impl Into<IncomingConnection>,
) -> eros::Result<Option<PendingConnectionRequest>> {
    match connection.into() {
        IncomingConnection::Quic(connection) => receive_quic_request(connection)
            .await
            .map(|request| request.map(PendingConnectionRequest::Quic)),
        IncomingConnection::Tcp {
            stream,
            remote_address,
            endpoint_identity,
        } => receive_tcp_request(stream, remote_address, endpoint_identity)
            .await
            .map(|request| request.map(PendingConnectionRequest::Tcp)),
    }
}

async fn receive_quic_request(
    connection: compio::quic::Connection,
) -> eros::Result<Option<PendingQuicConnectionRequest>> {
    let remote_address = connection.remote_address();
    let started_at = Instant::now();
    let (response_stream, mut request_stream) = match connection.accept_bi().await {
        Ok(streams) => streams,
        Err(compio::quic::ConnectionError::ApplicationClosed(close))
            if close.reason.as_ref()
                == crate::infra::quic_endpoint::SELF_CONNECTION_CLOSE_REASON =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Ok(
                Err(error).with_context(|| "Failed to accept QUIC connection request stream")?
            );
        }
    };
    let request = recv_quic_request(&mut request_stream).await?;

    info!(
        event = "connection_request_received",
        transport = "quic",
        %remote_address,
        requester_name = %request.requester_name,
        "Connection request received"
    );
    debug!(
        %remote_address,
        elapsed_ms = started_at.elapsed().as_millis(),
        stats = ?connection.stats(),
        "Received QUIC connection request"
    );

    Ok(Some(PendingQuicConnectionRequest {
        request,
        remote_address,
        connection,
        response_stream,
    }))
}

async fn receive_tcp_request(
    mut stream: compio::net::TcpStream,
    remote_address: SocketAddr,
    endpoint_identity: [u8; TCP_ENDPOINT_IDENTITY_SIZE],
) -> eros::Result<Option<PendingTcpConnectionRequest>> {
    let started_at = Instant::now();
    let (peer_identity, request) = recv_tcp_request_message(&mut stream).await?;

    if peer_identity == endpoint_identity {
        send_tcp_response(&mut stream, ConnectionResponse::SelfConnection).await?;
        stream
            .shutdown()
            .await
            .with_context(|| "Failed to close rejected TCP self-connection")?;
        info!(
            event = "self_connection_rejected",
            %remote_address,
            "Self-connection rejected"
        );
        return Ok(None);
    }

    info!(
        event = "connection_request_received",
        transport = "tcp",
        %remote_address,
        requester_name = %request.requester_name,
        "Connection request received"
    );
    debug!(
        %remote_address,
        elapsed_ms = started_at.elapsed().as_millis(),
        "Received TCP connection request"
    );

    Ok(Some(PendingTcpConnectionRequest {
        request,
        remote_address,
        stream,
    }))
}

impl PendingConnectionRequest {
    pub(crate) fn request(&self) -> &ConnectionRequest {
        match self {
            Self::Quic(request) => &request.request,
            Self::Tcp(request) => &request.request,
        }
    }

    pub(crate) fn remote_address(&self) -> SocketAddr {
        match self {
            Self::Quic(request) => request.remote_address,
            Self::Tcp(request) => request.remote_address,
        }
    }

    pub(crate) async fn accept(self) -> eros::Result<SessionTransport> {
        match self {
            Self::Quic(mut request) => {
                send_quic_response(&mut request.response_stream, ConnectionResponse::Accepted)
                    .await?;
                Ok(SessionTransport::Quic(
                    QuicTransport::accept(request.connection).await?,
                ))
            }
            Self::Tcp(mut request) => {
                send_tcp_response(&mut request.stream, ConnectionResponse::Accepted).await?;
                Ok(SessionTransport::Tcp(TcpTransport::new(request.stream)?))
            }
        }
    }

    pub(crate) async fn reject(self) -> eros::Result<()> {
        match self {
            Self::Quic(mut request) => {
                send_quic_response(&mut request.response_stream, ConnectionResponse::Rejected)
                    .await?;
                request.response_stream.stopped().await.with_context(
                    || "Failed while confirming the rejected QUIC connection response",
                )?;
                request.connection.close(
                    compio::quic::VarInt::from_u32(0),
                    b"Connection request rejected",
                );
            }
            Self::Tcp(mut request) => {
                send_tcp_response(&mut request.stream, ConnectionResponse::Rejected).await?;
                request
                    .stream
                    .shutdown()
                    .await
                    .with_context(|| "Failed to close rejected TCP connection")?;
            }
        }

        Ok(())
    }
}

fn log_response(remote_address: SocketAddr, response: ConnectionResponse) {
    let decision = match response {
        ConnectionResponse::Accepted => "accepted",
        ConnectionResponse::Rejected => "rejected",
        ConnectionResponse::SelfConnection => "self_connection",
    };
    info!(
        event = "connection_response_received",
        %remote_address,
        decision,
        "Connection response received"
    );
}

async fn send_quic_request(
    stream: &mut compio::quic::SendStream,
    request: ConnectionRequest,
) -> eros::Result<()> {
    let requester_name = Bytes::from(request.requester_name);
    let requester_name_length = u16::try_from(requester_name.len())
        .with_context(|| "Failed to encode connection requester name length")?;
    let mut header = BytesMut::with_capacity(REQUESTER_NAME_LENGTH_SIZE);

    header.put_u16(requester_name_length);

    let mut chunks = [header.freeze(), requester_name];

    stream
        .write_all_chunks(&mut chunks)
        .await
        .with_context(|| "Failed to send QUIC connection request")?;
    stream
        .finish()
        .with_context(|| "Failed to finish QUIC connection request stream")?;

    Ok(())
}

async fn recv_quic_request(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<ConnectionRequest> {
    let header = read_quic_exact(stream, REQUESTER_NAME_LENGTH_SIZE)
        .await
        .with_context(|| "Failed to receive QUIC connection request header")?;
    let requester_name_length = usize::from(u16::from_be_bytes([header[0], header[1]]));
    let requester_name = read_quic_exact(stream, requester_name_length)
        .await
        .with_context(|| "Failed to receive QUIC connection requester name")?;
    let requester_name = String::from_utf8(requester_name.into())
        .with_context(|| "Failed to decode QUIC connection requester name as UTF-8")?;

    Ok(ConnectionRequest { requester_name })
}

async fn send_quic_response(
    stream: &mut compio::quic::SendStream,
    response: ConnectionResponse,
) -> eros::Result<()> {
    let mut response = [Bytes::copy_from_slice(&[response.into()])];

    stream
        .write_all_chunks(&mut response)
        .await
        .with_context(|| "Failed to send QUIC connection response")?;
    stream
        .finish()
        .with_context(|| "Failed to finish QUIC connection response stream")?;

    Ok(())
}

async fn recv_quic_response(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<ConnectionResponse> {
    let response = read_quic_exact(stream, RESPONSE_SIZE)
        .await
        .with_context(|| "Failed to receive QUIC connection response")?;

    Ok(ConnectionResponse::try_from(response[0])
        .with_context(|| "Failed to decode QUIC connection response")?)
}

async fn read_quic_exact(
    stream: &mut compio::quic::RecvStream,
    length: usize,
) -> eros::Result<BytesMut> {
    let mut buffer = BytesMut::with_capacity(length);

    while buffer.len() < length {
        let remaining = length - buffer.len();
        let Some(chunk) = stream
            .read_chunk(remaining, true)
            .await
            .with_context(|| "Failed to read QUIC connection request stream")?
        else {
            eros::bail!("QUIC connection request stream ended before the message was complete");
        };

        buffer.extend_from_slice(&chunk.bytes);
    }

    Ok(buffer)
}

async fn send_tcp_request(
    stream: &mut compio::net::TcpStream,
    endpoint_identity: [u8; TCP_ENDPOINT_IDENTITY_SIZE],
    request: ConnectionRequest,
) -> eros::Result<()> {
    let requester_name = request.requester_name.into_bytes();
    let requester_name_length = u16::try_from(requester_name.len())
        .with_context(|| "Failed to encode TCP connection requester name length")?;
    let mut message = BytesMut::with_capacity(
        TCP_REQUEST_MAGIC.len()
            + TCP_ENDPOINT_IDENTITY_SIZE
            + REQUESTER_NAME_LENGTH_SIZE
            + requester_name.len(),
    );
    message.extend_from_slice(TCP_REQUEST_MAGIC);
    message.extend_from_slice(&endpoint_identity);
    message.put_u16(requester_name_length);
    message.extend_from_slice(&requester_name);

    Ok(stream
        .write_all(message.freeze())
        .await
        .0
        .with_context(|| "Failed to send TCP connection request")?)
}

async fn recv_tcp_request_message(
    stream: &mut compio::net::TcpStream,
) -> eros::Result<([u8; TCP_ENDPOINT_IDENTITY_SIZE], ConnectionRequest)> {
    let header_length =
        TCP_REQUEST_MAGIC.len() + TCP_ENDPOINT_IDENTITY_SIZE + REQUESTER_NAME_LENGTH_SIZE;
    let header = read_tcp_exact(stream, header_length, "TCP connection request header").await?;
    if &header[..TCP_REQUEST_MAGIC.len()] != TCP_REQUEST_MAGIC {
        eros::bail!("TCP connection request has an invalid protocol preface");
    }
    let identity_start = TCP_REQUEST_MAGIC.len();
    let identity_end = identity_start + TCP_ENDPOINT_IDENTITY_SIZE;
    let mut endpoint_identity = [0; TCP_ENDPOINT_IDENTITY_SIZE];
    endpoint_identity.copy_from_slice(&header[identity_start..identity_end]);
    let requester_name_length = usize::from(u16::from_be_bytes([
        header[identity_end],
        header[identity_end + 1],
    ]));
    let requester_name = read_tcp_exact(
        stream,
        requester_name_length,
        "TCP connection requester name",
    )
    .await?;
    let requester_name = String::from_utf8(requester_name)
        .with_context(|| "Failed to decode TCP connection requester name as UTF-8")?;

    Ok((endpoint_identity, ConnectionRequest { requester_name }))
}

async fn send_tcp_response(
    stream: &mut compio::net::TcpStream,
    response: ConnectionResponse,
) -> eros::Result<()> {
    Ok(stream
        .write_all(Bytes::copy_from_slice(&[response.into()]))
        .await
        .0
        .with_context(|| "Failed to send TCP connection response")?)
}

async fn recv_tcp_response(
    stream: &mut compio::net::TcpStream,
) -> eros::Result<ConnectionResponse> {
    let response = read_tcp_exact(stream, RESPONSE_SIZE, "TCP connection response").await?;

    Ok(ConnectionResponse::try_from(response[0])
        .with_context(|| "Failed to decode TCP connection response")?)
}

async fn read_tcp_exact(
    stream: &mut compio::net::TcpStream,
    length: usize,
    operation: &'static str,
) -> eros::Result<Vec<u8>> {
    let result = stream.read_exact(Vec::with_capacity(length)).await;
    result
        .0
        .with_context(|| format!("Failed to receive {operation}"))?;
    Ok(result.1)
}

#[cfg(test)]
mod tests {
    use crate::{
        infra::{
            ConnectionEndpoint, DirectConnectionOutcome, IncomingConnection, TcpEndpoint,
            connect_transport, receive_request,
        },
        kernel::connection_request::ConnectionRequest,
    };

    #[test]
    fn tcp_connection_request_establishes_transport_after_approval() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let outgoing = TcpEndpoint::new_for_test()
                .await
                .expect("Outgoing test TCP endpoint should start");
            let incoming = TcpEndpoint::new_for_test()
                .await
                .expect("Incoming test TCP endpoint should start");
            let incoming_address = incoming
                .local_address()
                .expect("Incoming test TCP endpoint address should be available");
            let incoming_task = compio::runtime::spawn(async move {
                let (stream, remote_address) = incoming
                    .accept_connection()
                    .await
                    .expect("TCP connection should be accepted");
                let request = receive_request(IncomingConnection::Tcp {
                    stream,
                    remote_address,
                    endpoint_identity: incoming.identity(),
                })
                .await
                .expect("TCP connection request should be received")
                .expect("TCP connection request should require approval");
                assert_eq!(request.request().requester_name, "outgoing");
                request
                    .accept()
                    .await
                    .expect("TCP connection request should be accepted")
            });
            let outcome = connect_transport(
                &ConnectionEndpoint::Tcp(outgoing),
                incoming_address.ip(),
                Some(incoming_address.port()),
                ConnectionRequest {
                    requester_name: "outgoing".to_string(),
                },
            )
            .await
            .expect("TCP connection request should complete");
            let DirectConnectionOutcome::Connected(_) = outcome else {
                panic!("TCP connection request should establish a transport");
            };
            incoming_task
                .await
                .expect("Incoming TCP approval task should finish");
        });
    }

    #[test]
    fn tcp_endpoint_rejects_its_own_connection_before_approval() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let endpoint = TcpEndpoint::new_for_test()
                .await
                .expect("Test TCP endpoint should start");
            let address = endpoint
                .local_address()
                .expect("Test TCP endpoint address should be available");
            let incoming_endpoint = endpoint.clone();
            let incoming_task = compio::runtime::spawn(async move {
                let (stream, remote_address) = incoming_endpoint
                    .accept_connection()
                    .await
                    .expect("TCP self-connection should be accepted at socket level");
                receive_request(IncomingConnection::Tcp {
                    stream,
                    remote_address,
                    endpoint_identity: incoming_endpoint.identity(),
                })
                .await
                .expect("TCP self-connection should be handled")
            });
            let outcome = connect_transport(
                &ConnectionEndpoint::Tcp(endpoint),
                address.ip(),
                Some(address.port()),
                ConnectionRequest {
                    requester_name: "self".to_string(),
                },
            )
            .await
            .expect("TCP self-connection should receive a response");

            assert!(matches!(outcome, DirectConnectionOutcome::SelfConnection));
            assert!(
                incoming_task
                    .await
                    .expect("Incoming TCP self-connection task should finish")
                    .is_none()
            );
        });
    }
}

// Focused test: cargo test infra::connection_request::tests:: --lib
