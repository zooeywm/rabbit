use crate::kernel::connection_request::{ConnectionRequest, ConnectionResponse};
use bytes::{BufMut, Bytes, BytesMut};
use eros::Context;

const REQUESTER_NAME_LENGTH_SIZE: usize = size_of::<u16>();
const RESPONSE_SIZE: usize = size_of::<u8>();

pub(crate) async fn send_request(
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

pub(crate) async fn recv_request(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<ConnectionRequest> {
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

pub(crate) async fn send_response(
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

pub(crate) async fn recv_response(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<ConnectionResponse> {
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
