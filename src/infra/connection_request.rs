use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use bytes::{BufMut, Bytes, BytesMut};
use eros::Context;
use tracing::{debug, info};

use crate::{
    infra::{QuicEndpoint, QuicTransport},
    kernel::connection_request::{ConnectionRequest, ConnectionResponse},
};

const REQUESTER_NAME_LENGTH_SIZE: usize = size_of::<u16>();
const RESPONSE_SIZE: usize = size_of::<u8>();

pub(crate) struct PendingQuicConnectionRequest {
    request: ConnectionRequest,
    remote_address: SocketAddr,
    connection: compio::quic::Connection,
    response_stream: compio::quic::SendStream,
}

pub(crate) async fn connect_transport(
    endpoint: &QuicEndpoint,
    remote_ip: IpAddr,
    remote_port: Option<u16>,
    request: ConnectionRequest,
) -> eros::Result<Option<QuicTransport>> {
    if let Some(remote_port) = remote_port {
        let remote_address = SocketAddr::new(remote_ip, remote_port);
        let connection = endpoint.connect(remote_address).await?;

        return request_transport(connection, request).await;
    }

    let mut last_error = None;

    for remote_port in QuicEndpoint::default_ports() {
        let remote_address = SocketAddr::new(remote_ip, remote_port);

        match endpoint.connect(remote_address).await {
            Ok(connection) => return request_transport(connection, request).await,
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

pub(crate) async fn request_transport(
    connection: compio::quic::Connection,
    request: ConnectionRequest,
) -> eros::Result<Option<QuicTransport>> {
    let remote_address = connection.remote_address();
    let (mut request_stream, mut response_stream) = connection
        .open_bi_wait()
        .await
        .with_context(|| "Failed to open QUIC connection request stream")?;

    send_request(&mut request_stream, request).await?;

    let response = recv_response(&mut response_stream).await?;
    let decision = match response {
        ConnectionResponse::Accepted => "accepted",
        ConnectionResponse::Rejected => "rejected",
    };
    info!(
        event = "connection_response_received",
        %remote_address,
        decision,
        "Connection response received"
    );

    match response {
        ConnectionResponse::Accepted => Ok(Some(QuicTransport::open(connection).await?)),
        ConnectionResponse::Rejected => Ok(None),
    }
}

pub(crate) async fn receive_request(
    connection: compio::quic::Connection,
) -> eros::Result<PendingQuicConnectionRequest> {
    let remote_address = connection.remote_address();
    let started_at = Instant::now();
    let (response_stream, mut request_stream) = connection
        .accept_bi()
        .await
        .with_context(|| "Failed to accept QUIC connection request stream")?;
    let request = recv_request(&mut request_stream).await?;

    info!(
        event = "connection_request_received",
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

    Ok(PendingQuicConnectionRequest {
        request,
        remote_address,
        connection,
        response_stream,
    })
}

impl PendingQuicConnectionRequest {
    pub(crate) fn request(&self) -> &ConnectionRequest {
        &self.request
    }

    pub(crate) fn remote_address(&self) -> SocketAddr {
        self.remote_address
    }

    pub(crate) async fn accept(mut self) -> eros::Result<QuicTransport> {
        send_response(&mut self.response_stream, ConnectionResponse::Accepted).await?;

        QuicTransport::accept(self.connection).await
    }

    pub(crate) async fn reject(mut self) -> eros::Result<()> {
        send_response(&mut self.response_stream, ConnectionResponse::Rejected).await?;
        self.response_stream
            .stopped()
            .await
            .with_context(|| "Failed while confirming the rejected QUIC connection response")?;
        self.connection.close(
            compio::quic::VarInt::from_u32(0),
            b"Connection request rejected",
        );

        Ok(())
    }
}

async fn send_request(
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

async fn recv_request(stream: &mut compio::quic::RecvStream) -> eros::Result<ConnectionRequest> {
    let header = read_exact(stream, REQUESTER_NAME_LENGTH_SIZE)
        .await
        .with_context(|| "Failed to receive QUIC connection request header")?;
    let requester_name_length = usize::from(u16::from_be_bytes([header[0], header[1]]));
    let requester_name = read_exact(stream, requester_name_length)
        .await
        .with_context(|| "Failed to receive QUIC connection requester name")?;
    let requester_name = String::from_utf8(requester_name.into())
        .with_context(|| "Failed to decode QUIC connection requester name as UTF-8")?;

    Ok(ConnectionRequest { requester_name })
}

async fn send_response(
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

async fn recv_response(stream: &mut compio::quic::RecvStream) -> eros::Result<ConnectionResponse> {
    let response = read_exact(stream, RESPONSE_SIZE)
        .await
        .with_context(|| "Failed to receive QUIC connection response")?;

    Ok(ConnectionResponse::try_from(response[0])
        .with_context(|| "Failed to decode QUIC connection response")?)
}

async fn read_exact(
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
