use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use bytes::{BufMut, Bytes, BytesMut};
use compio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio::net::ToSocketAddrsAsync;
use eros::Context;
use tracing::{debug, info};

use crate::{
    infra::{
        ConnectionEndpoint, IncomingConnection, QuicConnectOutcome, QuicEndpoint, SessionTransport,
        TcpEndpoint,
        transport::{QuicTransport, TcpTransport},
    },
    kernel::{
        connection_request::{
            ConnectionHandshakeReply, ConnectionRequest, ConnectionResponse, EncoderProfileTag,
            PeerCapabilities,
        },
        protocol::{PROTOCOL_MAJOR, PROTOCOL_MINOR},
    },
};

const REQUESTER_NAME_LENGTH_SIZE: usize = size_of::<u16>();
const PROTOCOL_VERSION_SIZE: usize = size_of::<u16>() * 2;
const CAPABILITY_HEADER_SIZE: usize = size_of::<u8>() * 2;
const CAPABILITY_TAG_ABSOLUTE_POINTER: u8 = 0x80;
const RESPONSE_SIZE: usize = size_of::<u8>();
/// TCP handshake preface for protocol-aware connection requests.
const TCP_REQUEST_MAGIC: &[u8; 5] = b"RBTC\x02";
const TCP_ENDPOINT_IDENTITY_SIZE: usize = 16;
const MAX_CAPABILITY_TAGS: usize = 16;
const MAX_REQUESTER_NAME_BYTES: usize = 512;

pub(crate) enum DirectConnectionOutcome {
    Connected {
        transport: SessionTransport,
        host_capabilities: PeerCapabilities,
    },
    Rejected,
    SelfConnection,
    ProtocolMismatch {
        peer_major: u16,
        peer_minor: u16,
    },
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
    remote_host: String,
    remote_port: Option<u16>,
    request: ConnectionRequest,
) -> eros::Result<DirectConnectionOutcome> {
    let resolved = (remote_host.clone(), remote_port.unwrap_or(0))
        .to_socket_addrs_async()
        .await
        .with_context(|| format!("Failed to resolve Rabbit host {remote_host}"))?;
    let mut remote_ips = Vec::new();
    for address in resolved {
        if !remote_ips.contains(&address.ip()) {
            remote_ips.push(address.ip());
        }
    }
    let mut last_error = None;

    for remote_ip in remote_ips {
        match connect_resolved_transport(endpoint, remote_ip, remote_port, request.clone()).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => last_error = Some(error),
        }
    }

    let Some(last_error) = last_error else {
        eros::bail!("Rabbit host {} resolved to no IP addresses", remote_host);
    };

    Err(last_error).with_context(|| format!("Failed to connect resolved Rabbit host {remote_host}"))
}

async fn connect_resolved_transport(
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

    let local_major = request.protocol_major;
    let local_minor = request.protocol_minor;
    send_quic_request(&mut request_stream, request).await?;

    let response = recv_quic_response(&mut response_stream).await?;
    log_response(remote_address, &response);

    match response {
        ConnectionHandshakeReply::Accepted { host_capabilities } => {
            Ok(DirectConnectionOutcome::Connected {
                transport: SessionTransport::Quic(QuicTransport::open(connection).await?),
                host_capabilities,
            })
        }
        ConnectionHandshakeReply::Rejected => Ok(DirectConnectionOutcome::Rejected),
        ConnectionHandshakeReply::SelfConnection => Ok(DirectConnectionOutcome::SelfConnection),
        ConnectionHandshakeReply::ProtocolMismatch => {
            Ok(DirectConnectionOutcome::ProtocolMismatch {
                peer_major: local_major,
                peer_minor: local_minor,
            })
        }
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
    let local_major = request.protocol_major;
    let local_minor = request.protocol_minor;
    send_tcp_request(&mut stream, endpoint_identity, request).await?;
    let response = recv_tcp_response(&mut stream).await?;
    log_response(remote_address, &response);

    match response {
        ConnectionHandshakeReply::Accepted { host_capabilities } => {
            Ok(DirectConnectionOutcome::Connected {
                transport: SessionTransport::Tcp(TcpTransport::new(stream)?),
                host_capabilities,
            })
        }
        ConnectionHandshakeReply::Rejected => Ok(DirectConnectionOutcome::Rejected),
        ConnectionHandshakeReply::SelfConnection => Ok(DirectConnectionOutcome::SelfConnection),
        ConnectionHandshakeReply::ProtocolMismatch => {
            Ok(DirectConnectionOutcome::ProtocolMismatch {
                peer_major: local_major,
                peer_minor: local_minor,
            })
        }
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
    if !request.is_protocol_compatible() {
        warn_protocol_mismatch(remote_address, &request);
        let mut response_stream = response_stream;
        send_quic_response(
            &mut response_stream,
            &ConnectionHandshakeReply::ProtocolMismatch,
        )
        .await?;
        connection.close(
            compio::quic::VarInt::from_u32(0),
            b"Protocol major version mismatch",
        );
        return Ok(None);
    }

    info!(
        event = "connection_request_received",
        transport = "quic",
        %remote_address,
        requester_name = %request.requester_name,
        protocol_major = request.protocol_major,
        protocol_minor = request.protocol_minor,
        max_screens = request.capabilities.max_screens,
        encoder_profiles = request.capabilities.encoder_profiles.len(),
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
        send_tcp_response(&mut stream, &ConnectionHandshakeReply::SelfConnection).await?;
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

    if !request.is_protocol_compatible() {
        warn_protocol_mismatch(remote_address, &request);
        send_tcp_response(&mut stream, &ConnectionHandshakeReply::ProtocolMismatch).await?;
        stream
            .shutdown()
            .await
            .with_context(|| "Failed to close protocol-mismatched TCP connection")?;
        return Ok(None);
    }

    info!(
        event = "connection_request_received",
        transport = "tcp",
        %remote_address,
        requester_name = %request.requester_name,
        protocol_major = request.protocol_major,
        protocol_minor = request.protocol_minor,
        max_screens = request.capabilities.max_screens,
        encoder_profiles = request.capabilities.encoder_profiles.len(),
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

    pub(crate) async fn accept(
        self,
        host_capabilities: PeerCapabilities,
    ) -> eros::Result<SessionTransport> {
        let reply = ConnectionHandshakeReply::Accepted { host_capabilities };
        match self {
            Self::Quic(mut request) => {
                send_quic_response(&mut request.response_stream, &reply).await?;
                Ok(SessionTransport::Quic(
                    QuicTransport::accept(request.connection).await?,
                ))
            }
            Self::Tcp(mut request) => {
                send_tcp_response(&mut request.stream, &reply).await?;
                Ok(SessionTransport::Tcp(TcpTransport::new(request.stream)?))
            }
        }
    }

    pub(crate) async fn reject(self) -> eros::Result<()> {
        match self {
            Self::Quic(mut request) => {
                send_quic_response(
                    &mut request.response_stream,
                    &ConnectionHandshakeReply::Rejected,
                )
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
                send_tcp_response(&mut request.stream, &ConnectionHandshakeReply::Rejected).await?;
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

fn log_response(remote_address: SocketAddr, response: &ConnectionHandshakeReply) {
    let decision = match response {
        ConnectionHandshakeReply::Accepted { .. } => "accepted",
        ConnectionHandshakeReply::Rejected => "rejected",
        ConnectionHandshakeReply::SelfConnection => "self_connection",
        ConnectionHandshakeReply::ProtocolMismatch => "protocol_mismatch",
    };
    info!(
        event = "connection_response_received",
        %remote_address,
        decision,
        "Connection response received"
    );
}

fn warn_protocol_mismatch(remote_address: SocketAddr, request: &ConnectionRequest) {
    tracing::warn!(
        event = "connection_protocol_mismatch",
        %remote_address,
        peer_major = request.protocol_major,
        peer_minor = request.protocol_minor,
        local_major = PROTOCOL_MAJOR,
        local_minor = PROTOCOL_MINOR,
        "Rejected connection request for protocol major mismatch"
    );
}

fn encode_peer_capabilities(capabilities: &PeerCapabilities) -> eros::Result<BytesMut> {
    let tag_count =
        capabilities.encoder_profiles.len() + usize::from(capabilities.absolute_pointer);
    if tag_count > MAX_CAPABILITY_TAGS {
        eros::bail!(
            "Connection advertises more than {} capability tags",
            MAX_CAPABILITY_TAGS
        );
    }
    let profile_count =
        u8::try_from(tag_count).with_context(|| "Failed to encode capability tag count")?;
    let mut body = BytesMut::with_capacity(CAPABILITY_HEADER_SIZE + tag_count);
    body.put_u8(capabilities.max_screens);
    body.put_u8(profile_count);
    for profile in &capabilities.encoder_profiles {
        body.put_u8(profile.as_u8());
    }
    if capabilities.absolute_pointer {
        body.put_u8(CAPABILITY_TAG_ABSOLUTE_POINTER);
    }
    Ok(body)
}

fn decode_peer_capabilities(mut body: Bytes) -> eros::Result<(PeerCapabilities, Bytes)> {
    if body.len() < CAPABILITY_HEADER_SIZE {
        eros::bail!("Capability header is too short ({} bytes)", body.len());
    }
    let max_screens = body[0];
    let profile_count = usize::from(body[1]);
    body = body.split_off(CAPABILITY_HEADER_SIZE);
    if profile_count > MAX_CAPABILITY_TAGS {
        eros::bail!("Advertises {profile_count} capability tags (max {MAX_CAPABILITY_TAGS})");
    }
    if body.len() < profile_count {
        eros::bail!("Truncated while reading capability tags");
    }
    let mut encoder_profiles = Vec::with_capacity(profile_count);
    let mut absolute_pointer = false;
    for tag in body.split_to(profile_count) {
        if tag == CAPABILITY_TAG_ABSOLUTE_POINTER {
            absolute_pointer = true;
        } else if let Ok(profile) = EncoderProfileTag::try_from(tag) {
            encoder_profiles.push(profile);
        }
    }
    Ok((
        PeerCapabilities {
            max_screens,
            encoder_profiles,
            absolute_pointer,
        },
        body,
    ))
}

fn encode_connection_request_body(request: &ConnectionRequest) -> eros::Result<Bytes> {
    if request.requester_name.len() > MAX_REQUESTER_NAME_BYTES {
        eros::bail!(
            "Connection requester name exceeds {} bytes",
            MAX_REQUESTER_NAME_BYTES
        );
    }

    let requester_name = request.requester_name.as_bytes();
    let requester_name_length = u16::try_from(requester_name.len())
        .with_context(|| "Failed to encode connection requester name length")?;
    let caps = encode_peer_capabilities(&request.capabilities)?;
    let mut body = BytesMut::with_capacity(
        PROTOCOL_VERSION_SIZE + caps.len() + REQUESTER_NAME_LENGTH_SIZE + requester_name.len(),
    );
    body.put_u16(request.protocol_major);
    body.put_u16(request.protocol_minor);
    body.extend_from_slice(&caps);
    body.put_u16(requester_name_length);
    body.extend_from_slice(requester_name);
    Ok(body.freeze())
}

fn decode_connection_request_body(mut body: Bytes) -> eros::Result<ConnectionRequest> {
    if body.len() < PROTOCOL_VERSION_SIZE + CAPABILITY_HEADER_SIZE + REQUESTER_NAME_LENGTH_SIZE {
        eros::bail!(
            "Connection request body is too short ({} bytes)",
            body.len()
        );
    }

    let protocol_major = u16::from_be_bytes([body[0], body[1]]);
    let protocol_minor = u16::from_be_bytes([body[2], body[3]]);
    body = body.split_off(PROTOCOL_VERSION_SIZE);
    let (capabilities, mut body) = decode_peer_capabilities(body)?;

    if body.len() < REQUESTER_NAME_LENGTH_SIZE {
        eros::bail!("Connection request truncated while reading requester name length");
    }
    let requester_name_length = usize::from(u16::from_be_bytes([body[0], body[1]]));
    body = body.split_off(REQUESTER_NAME_LENGTH_SIZE);
    if requester_name_length > MAX_REQUESTER_NAME_BYTES {
        eros::bail!("Connection requester name length {requester_name_length} exceeds limit");
    }
    if body.len() != requester_name_length {
        eros::bail!(
            "Connection request name length {requester_name_length} does not match remaining {} bytes",
            body.len()
        );
    }
    let requester_name = String::from_utf8(body.to_vec())
        .with_context(|| "Failed to decode connection requester name as UTF-8")?;

    Ok(ConnectionRequest {
        protocol_major,
        protocol_minor,
        requester_name,
        capabilities,
    })
}

fn encode_connection_reply(reply: &ConnectionHandshakeReply) -> eros::Result<Bytes> {
    match reply {
        ConnectionHandshakeReply::Accepted { host_capabilities } => {
            let caps = encode_peer_capabilities(host_capabilities)?;
            let mut body = BytesMut::with_capacity(1 + caps.len());
            body.put_u8(ConnectionResponse::Accepted.into());
            body.extend_from_slice(&caps);
            Ok(body.freeze())
        }
        other => Ok(Bytes::copy_from_slice(&[other.status().into()])),
    }
}

fn decode_connection_reply(mut body: Bytes) -> eros::Result<ConnectionHandshakeReply> {
    if body.is_empty() {
        eros::bail!("Connection response is empty");
    }
    let status = ConnectionResponse::try_from(body[0])
        .with_context(|| "Failed to decode connection response status")?;
    body = body.split_off(1);
    match status {
        ConnectionResponse::Accepted => {
            let (host_capabilities, rest) = decode_peer_capabilities(body)?;
            if !rest.is_empty() {
                eros::bail!(
                    "Accepted connection response has {} trailing bytes",
                    rest.len()
                );
            }
            Ok(ConnectionHandshakeReply::Accepted { host_capabilities })
        }
        ConnectionResponse::Rejected => Ok(ConnectionHandshakeReply::Rejected),
        ConnectionResponse::SelfConnection => Ok(ConnectionHandshakeReply::SelfConnection),
        ConnectionResponse::ProtocolMismatch => Ok(ConnectionHandshakeReply::ProtocolMismatch),
    }
}

async fn send_quic_request(
    stream: &mut compio::quic::SendStream,
    request: ConnectionRequest,
) -> eros::Result<()> {
    let body = encode_connection_request_body(&request)?;
    let mut chunks = [body];

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
    let mut body = BytesMut::new();
    loop {
        let Some(chunk) = stream
            .read_chunk(64 * 1024, true)
            .await
            .with_context(|| "Failed to read QUIC connection request stream")?
        else {
            break;
        };
        body.extend_from_slice(&chunk.bytes);
        if body.len() > 8 * 1024 {
            eros::bail!("QUIC connection request exceeds 8 KiB");
        }
    }

    decode_connection_request_body(body.freeze())
        .with_context(|| "Failed to decode QUIC connection request")
}

async fn send_quic_response(
    stream: &mut compio::quic::SendStream,
    reply: &ConnectionHandshakeReply,
) -> eros::Result<()> {
    let body = encode_connection_reply(reply)?;
    let mut chunks = [body];

    stream
        .write_all_chunks(&mut chunks)
        .await
        .with_context(|| "Failed to send QUIC connection response")?;
    stream
        .finish()
        .with_context(|| "Failed to finish QUIC connection response stream")?;

    Ok(())
}

async fn recv_quic_response(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<ConnectionHandshakeReply> {
    let mut body = BytesMut::new();
    loop {
        let Some(chunk) = stream
            .read_chunk(64 * 1024, true)
            .await
            .with_context(|| "Failed to receive QUIC connection response")?
        else {
            break;
        };
        body.extend_from_slice(&chunk.bytes);
        if body.len() > 4 * 1024 {
            eros::bail!("QUIC connection response exceeds 4 KiB");
        }
    }
    decode_connection_reply(body.freeze())
        .with_context(|| "Failed to decode QUIC connection response")
}

async fn send_tcp_request(
    stream: &mut compio::net::TcpStream,
    endpoint_identity: [u8; TCP_ENDPOINT_IDENTITY_SIZE],
    request: ConnectionRequest,
) -> eros::Result<()> {
    let body = encode_connection_request_body(&request)?;
    let mut message =
        BytesMut::with_capacity(TCP_REQUEST_MAGIC.len() + TCP_ENDPOINT_IDENTITY_SIZE + body.len());
    message.extend_from_slice(TCP_REQUEST_MAGIC);
    message.extend_from_slice(&endpoint_identity);
    message.extend_from_slice(&body);

    Ok(stream
        .write_all(message.freeze())
        .await
        .0
        .with_context(|| "Failed to send TCP connection request")?)
}

async fn recv_tcp_request_message(
    stream: &mut compio::net::TcpStream,
) -> eros::Result<([u8; TCP_ENDPOINT_IDENTITY_SIZE], ConnectionRequest)> {
    let preface_length = TCP_REQUEST_MAGIC.len() + TCP_ENDPOINT_IDENTITY_SIZE;
    let preface = read_tcp_exact(stream, preface_length, "TCP connection request preface").await?;
    if &preface[..TCP_REQUEST_MAGIC.len()] != TCP_REQUEST_MAGIC {
        eros::bail!("TCP connection request has an invalid protocol preface");
    }
    let identity_start = TCP_REQUEST_MAGIC.len();
    let identity_end = identity_start + TCP_ENDPOINT_IDENTITY_SIZE;
    let mut endpoint_identity = [0; TCP_ENDPOINT_IDENTITY_SIZE];
    endpoint_identity.copy_from_slice(&preface[identity_start..identity_end]);

    // Fixed header of the body before name: version(4) + caps(2) + profiles(N) + name_len(2) + name
    let fixed = read_tcp_exact(
        stream,
        PROTOCOL_VERSION_SIZE + CAPABILITY_HEADER_SIZE,
        "TCP connection request capability header",
    )
    .await?;
    let profile_count = usize::from(fixed[5]);
    if profile_count > MAX_CAPABILITY_TAGS {
        eros::bail!(
            "TCP connection request advertises {profile_count} capability tags (max {MAX_CAPABILITY_TAGS})"
        );
    }
    let profiles_and_name_len = read_tcp_exact(
        stream,
        profile_count + REQUESTER_NAME_LENGTH_SIZE,
        "TCP connection request profiles",
    )
    .await?;
    let name_len_offset = profile_count;
    let requester_name_length = usize::from(u16::from_be_bytes([
        profiles_and_name_len[name_len_offset],
        profiles_and_name_len[name_len_offset + 1],
    ]));
    if requester_name_length > MAX_REQUESTER_NAME_BYTES {
        eros::bail!("TCP connection requester name is too long");
    }
    let requester_name = read_tcp_exact(
        stream,
        requester_name_length,
        "TCP connection requester name",
    )
    .await?;

    let mut body =
        BytesMut::with_capacity(fixed.len() + profiles_and_name_len.len() + requester_name.len());
    body.extend_from_slice(&fixed);
    body.extend_from_slice(&profiles_and_name_len);
    body.extend_from_slice(&requester_name);
    let request = decode_connection_request_body(body.freeze())
        .with_context(|| "Failed to decode TCP connection request")?;

    Ok((endpoint_identity, request))
}

async fn send_tcp_response(
    stream: &mut compio::net::TcpStream,
    reply: &ConnectionHandshakeReply,
) -> eros::Result<()> {
    let body = encode_connection_reply(reply)?;
    Ok(stream
        .write_all(body)
        .await
        .0
        .with_context(|| "Failed to send TCP connection response")?)
}

async fn recv_tcp_response(
    stream: &mut compio::net::TcpStream,
) -> eros::Result<ConnectionHandshakeReply> {
    let status = read_tcp_exact(stream, RESPONSE_SIZE, "TCP connection response status").await?;
    let status = ConnectionResponse::try_from(status[0])
        .with_context(|| "Failed to decode TCP connection response status")?;
    match status {
        ConnectionResponse::Accepted => {
            let header =
                read_tcp_exact(stream, CAPABILITY_HEADER_SIZE, "TCP host capability header")
                    .await?;
            let profile_count = usize::from(header[1]);
            if profile_count > MAX_CAPABILITY_TAGS {
                eros::bail!(
                    "Accepted TCP response advertises {profile_count} capability tags (max {MAX_CAPABILITY_TAGS})"
                );
            }
            let profiles =
                read_tcp_exact(stream, profile_count, "TCP host capability tags").await?;
            let mut body = BytesMut::with_capacity(1 + header.len() + profiles.len());
            body.put_u8(ConnectionResponse::Accepted.into());
            body.extend_from_slice(&header);
            body.extend_from_slice(&profiles);
            decode_connection_reply(body.freeze())
                .with_context(|| "Failed to decode accepted TCP connection response")
        }
        ConnectionResponse::Rejected => Ok(ConnectionHandshakeReply::Rejected),
        ConnectionResponse::SelfConnection => Ok(ConnectionHandshakeReply::SelfConnection),
        ConnectionResponse::ProtocolMismatch => Ok(ConnectionHandshakeReply::ProtocolMismatch),
    }
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
                assert!(request.request().is_protocol_compatible());
                assert!(!request.request().capabilities.encoder_profiles.is_empty());
                request
                    .accept(crate::kernel::connection_request::PeerCapabilities::local_host(2))
                    .await
                    .expect("TCP connection request should be accepted")
            });
            let outcome = connect_transport(
                &ConnectionEndpoint::Tcp(outgoing),
                "localhost".to_string(),
                Some(incoming_address.port()),
                ConnectionRequest::local(
                    "outgoing".to_string(),
                    crate::kernel::connection_request::PeerCapabilities::default(),
                ),
            )
            .await
            .expect("TCP connection request should complete");
            let DirectConnectionOutcome::Connected {
                host_capabilities, ..
            } = outcome
            else {
                panic!("TCP connection request should establish a transport");
            };
            assert_eq!(host_capabilities.max_screens, 2);
            assert!(host_capabilities.absolute_pointer);
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
                address.ip().to_string(),
                Some(address.port()),
                ConnectionRequest::local(
                    "self".to_string(),
                    crate::kernel::connection_request::PeerCapabilities::default(),
                ),
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
