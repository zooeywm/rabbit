use std::collections::VecDeque;

use crate::infra::unsync_queue::UnsyncQueue;
use crate::kernel::transport::{
    Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use eros::Context;
use thin_cell::unsync::ThinCell;

const TLV_HEADER_SIZE: usize = size_of::<u8>() + size_of::<u16>();
const MAX_DATAGRAM_BATCH_SIZE: usize = 256;

type ReceiveResult = eros::Result<Option<TransportMessage>>;
type ReceiveBatchResult = eros::Result<Option<VecDeque<TransportMessage>>>;

struct ReceiveItem {
    result: ReceiveBatchResult,
    consumed: UnsyncQueue<()>,
}

pub(crate) struct QuicTransport {
    connection: compio::quic::Connection,
    control_send: compio::quic::SendStream,
    control_recv: compio::quic::RecvStream,
}

pub(crate) struct QuicTransportSend {
    connection: compio::quic::Connection,
    control_commands: UnsyncQueue<ControlWriteCommand>,
    control_writer_closed: ThinCell<bool>,
    _control_writer: compio::runtime::JoinHandle<()>,
}

pub(crate) struct QuicTransportRecv {
    messages: UnsyncQueue<ReceiveItem>,
    pending: VecDeque<TransportMessage>,
    _tasks: [compio::runtime::JoinHandle<()>; 3],
}

struct ReliableStreamReader {
    stream: compio::quic::RecvStream,
    buffer: BytesMut,
    channel: Option<TransportChannel>,
}

struct ControlWriteCommand {
    channel: TransportChannel,
    payload: Bytes,
    completion: UnsyncQueue<eros::Result<()>>,
}

impl QuicTransport {
    pub(crate) fn remote_address(&self) -> std::net::SocketAddr {
        self.connection.remote_address()
    }

    pub(crate) async fn open(connection: compio::quic::Connection) -> eros::Result<Self> {
        let (mut control_send, mut control_recv) = connection
            .open_bi_wait()
            .await
            .with_context(|| "Failed to open QUIC Control stream")?;

        write_control_stream_preface(&mut control_send).await?;
        read_control_stream_preface(&mut control_recv).await?;

        Ok(Self {
            connection,
            control_send,
            control_recv,
        })
    }

    pub(crate) async fn accept(connection: compio::quic::Connection) -> eros::Result<Self> {
        let (mut control_send, mut control_recv) = connection
            .accept_bi()
            .await
            .with_context(|| "Failed to accept QUIC Control stream")?;

        read_control_stream_preface(&mut control_recv).await?;
        write_control_stream_preface(&mut control_send).await?;

        Ok(Self {
            connection,
            control_send,
            control_recv,
        })
    }
}

async fn write_control_stream_preface(stream: &mut compio::quic::SendStream) -> eros::Result<()> {
    let mut preface = [Bytes::copy_from_slice(&[TransportChannel::Control.into()])];

    stream
        .write_all_chunks(&mut preface)
        .await
        .with_context(|| "Failed to write QUIC Control stream preface")?;

    Ok(())
}

async fn read_control_stream_preface(stream: &mut compio::quic::RecvStream) -> eros::Result<()> {
    let Some(preface) = stream
        .read_chunk(size_of::<u8>(), true)
        .await
        .with_context(|| "Failed to read QUIC Control stream preface")?
    else {
        eros::bail!("QUIC Control stream ended before its preface");
    };
    let channel = TransportChannel::from(preface.bytes[0]);

    if channel != TransportChannel::Control {
        eros::bail!("QUIC Control stream has invalid preface channel {channel:?}");
    }

    Ok(())
}

impl Transport for QuicTransport {
    type SendHalf = QuicTransportSend;
    type RecvHalf = QuicTransportRecv;

    fn split(self) -> (Self::SendHalf, Self::RecvHalf) {
        let messages = UnsyncQueue::default();
        let control_commands = UnsyncQueue::default();
        let control_writer_closed = ThinCell::new(false);
        let send = QuicTransportSend {
            connection: self.connection.clone(),
            control_commands: control_commands.clone(),
            control_writer_closed: control_writer_closed.clone(),
            _control_writer: compio::runtime::spawn(run_control_writer(
                self.control_send,
                control_commands,
                control_writer_closed,
            )),
        };
        let recv = QuicTransportRecv {
            messages: messages.clone(),
            pending: VecDeque::new(),
            _tasks: [
                compio::runtime::spawn(receive_reliable_ordered(
                    ReliableStreamReader::from(self.control_recv),
                    messages.clone(),
                )),
                compio::runtime::spawn(receive_reliable_unordered(
                    self.connection.clone(),
                    messages.clone(),
                )),
                compio::runtime::spawn(receive_unreliable(self.connection, messages)),
            ],
        };

        (send, recv)
    }
}

impl TransportRecv for QuicTransportRecv {
    fn recv(&mut self) -> impl Future<Output = eros::Result<Option<TransportMessage>>> {
        receive_next_message(&mut self.pending, &self.messages)
    }
}

impl TransportSend for QuicTransportSend {
    fn max_unreliable_payload_size(&self) -> Option<usize> {
        max_tlv_payload_size(self.connection.max_datagram_size())
    }

    fn is_closed_normally(&self) -> bool {
        is_normal_close_reason(self.connection.close_reason())
    }

    fn send_unreliable(
        &self,
        channel: TransportChannel,
        payload: Bytes,
    ) -> impl Future<Output = eros::Result<()>> {
        self.send_unreliable_message(channel, payload)
    }

    fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>> {
        self.send_message(message)
    }

    fn close(&self) -> impl Future<Output = ()> {
        close_connection(&self.connection)
    }
}

async fn receive_next_message(
    pending: &mut VecDeque<TransportMessage>,
    messages: &UnsyncQueue<ReceiveItem>,
) -> eros::Result<Option<TransportMessage>> {
    if let Some(message) = pending.pop_front() {
        return Ok(Some(message));
    }

    let item = messages.pop().await;
    item.consumed.push(());
    let Some(mut received) = item.result? else {
        return Ok(None);
    };
    let message = received
        .pop_front()
        .with_context(|| "QUIC receive task published an empty message batch")?;
    pending.append(&mut received);

    Ok(Some(message))
}

async fn close_connection(connection: &compio::quic::Connection) {
    connection.close(
        compio::quic::VarInt::from_u32(0),
        b"Session closed normally",
    );
    connection.closed().await;
}

fn is_normal_close_reason(reason: Option<compio::quic::ConnectionError>) -> bool {
    match reason {
        Some(compio::quic::ConnectionError::LocallyClosed) => true,
        Some(compio::quic::ConnectionError::ApplicationClosed(close)) => {
            close.error_code.into_inner() == 0
        }
        _ => false,
    }
}

impl QuicTransportSend {
    async fn send_message(&self, message: TransportMessage) -> eros::Result<()> {
        let TransportMessage {
            channel,
            delivery,
            payload,
        } = message;

        match delivery {
            Delivery::ReliableOrdered => self.send_reliable_ordered(channel, payload).await,
            Delivery::ReliableUnordered => self.send_reliable_unordered(channel, payload).await,
            Delivery::Unreliable => self.send_unreliable_message(channel, payload).await,
        }
    }

    async fn send_unreliable_message(
        &self,
        channel: TransportChannel,
        payload: Bytes,
    ) -> eros::Result<()> {
        let datagram =
            encode_tlv(channel, payload).with_context(|| "Failed to encode QUIC datagram")?;

        Ok(self
            .connection
            .send_datagram_wait(datagram)
            .await
            .with_context(|| "Failed to send QUIC datagram")?)
    }

    async fn send_reliable_ordered(
        &self,
        channel: TransportChannel,
        payload: Bytes,
    ) -> eros::Result<()> {
        if channel != TransportChannel::Control {
            eros::bail!("Reliable ordered delivery is unavailable for channel {channel:?}");
        }

        if *self.control_writer_closed.borrow() {
            eros::bail!("Reliable ordered QUIC Control writer is unavailable");
        }

        let completion = UnsyncQueue::default();

        self.control_commands.push(ControlWriteCommand {
            channel,
            payload,
            completion: completion.clone(),
        });

        completion.pop().await
    }

    async fn send_reliable_unordered(
        &self,
        channel: TransportChannel,
        payload: Bytes,
    ) -> eros::Result<()> {
        let header = encode_tlv_header(channel, payload.len())
            .with_context(|| "Failed to encode unordered reliable QUIC TLV header")?;
        let mut stream = self
            .connection
            .open_uni_wait()
            .await
            .with_context(|| "Failed to open unordered reliable QUIC stream")?;
        let mut chunks = [header, payload];

        stream
            .write_all_chunks(&mut chunks)
            .await
            .with_context(|| "Failed to write unordered reliable QUIC message")?;
        stream
            .finish()
            .with_context(|| "Failed to finish unordered reliable QUIC stream")?;

        Ok(())
    }
}

async fn run_control_writer(
    mut stream: compio::quic::SendStream,
    commands: UnsyncQueue<ControlWriteCommand>,
    writer_closed: ThinCell<bool>,
) {
    loop {
        let command = commands.pop().await;
        let result = write_control_message(&mut stream, command.channel, command.payload).await;
        let failed = result.is_err();

        if failed {
            *writer_closed.borrow() = true;
            command.completion.push(result);

            while let Some(command) = commands.try_pop() {
                command.completion.push(Err(eros::error!(
                    "Reliable ordered QUIC Control writer stopped after an earlier write failure"
                )));
            }

            return;
        }

        command.completion.push(result);
    }
}

async fn write_control_message(
    stream: &mut compio::quic::SendStream,
    channel: TransportChannel,
    payload: Bytes,
) -> eros::Result<()> {
    let header = encode_tlv_header(channel, payload.len())
        .with_context(|| "Failed to encode reliable QUIC Control TLV header")?;
    let mut chunks = [header, payload];

    stream
        .write_all_chunks(&mut chunks)
        .await
        .with_context(|| "Failed to write reliable QUIC Control message")?;

    Ok(())
}

async fn receive_reliable_ordered(
    mut reader: ReliableStreamReader,
    messages: UnsyncQueue<ReceiveItem>,
) {
    let consumed = UnsyncQueue::default();

    loop {
        let message = recv_reliable_ordered(&mut reader).await;
        let finished = !matches!(&message, Ok(Some(_)));

        publish_received(&messages, &consumed, single_message_batch(message)).await;

        if finished {
            return;
        }
    }
}

async fn recv_reliable_ordered(reader: &mut ReliableStreamReader) -> ReceiveResult {
    let message = reader
        .recv(Delivery::ReliableOrdered)
        .await
        .with_context(|| "Failed to receive reliable QUIC Control message")?;

    Ok(message)
}

async fn receive_reliable_unordered(
    connection: compio::quic::Connection,
    messages: UnsyncQueue<ReceiveItem>,
) {
    let consumed = UnsyncQueue::default();

    loop {
        let message = recv_reliable_unordered(&connection).await;
        let finished = !matches!(&message, Ok(Some(_)));

        publish_received(&messages, &consumed, single_message_batch(message)).await;

        if finished {
            return;
        }
    }
}

async fn receive_unreliable(
    connection: compio::quic::Connection,
    messages: UnsyncQueue<ReceiveItem>,
) {
    let consumed = UnsyncQueue::default();

    loop {
        let batch = recv_unreliable_batch(&connection).await;
        let finished = !matches!(&batch, Ok(Some(_)));

        publish_received(&messages, &consumed, batch).await;

        if finished {
            return;
        }
    }
}

fn single_message_batch(result: ReceiveResult) -> ReceiveBatchResult {
    result.map(|message| message.map(|message| VecDeque::from([message])))
}

async fn publish_received(
    messages: &UnsyncQueue<ReceiveItem>,
    consumed: &UnsyncQueue<()>,
    result: ReceiveBatchResult,
) {
    messages.push(ReceiveItem {
        result,
        consumed: consumed.clone(),
    });
    consumed.pop().await;
}

async fn recv_unreliable_batch(connection: &compio::quic::Connection) -> ReceiveBatchResult {
    let datagram = connection.recv_datagram().await;
    if datagram.as_ref().is_err_and(is_normal_connection_close) {
        return Ok(None);
    }
    let datagram = datagram.with_context(|| "Failed to receive QUIC datagram")?;
    let mut batch = VecDeque::with_capacity(MAX_DATAGRAM_BATCH_SIZE);
    batch.push_back(decode_unreliable_datagram(datagram)?);

    while batch.len() < MAX_DATAGRAM_BATCH_SIZE {
        let datagram = match connection.try_recv_datagram() {
            Ok(Some(datagram)) => datagram,
            Ok(None) => break,
            Err(error) if is_normal_connection_close(&error) => break,
            Err(error) => {
                return Ok(Err(error).with_context(|| "Failed to drain queued QUIC datagrams")?);
            }
        };
        batch.push_back(decode_unreliable_datagram(datagram)?);
    }

    Ok(Some(batch))
}

fn decode_unreliable_datagram(datagram: Bytes) -> eros::Result<TransportMessage> {
    decode_tlv(datagram, Delivery::Unreliable).with_context(|| "Failed to decode QUIC datagram")
}

async fn recv_reliable_unordered(connection: &compio::quic::Connection) -> ReceiveResult {
    let stream = connection.accept_uni().await;
    if stream.as_ref().is_err_and(is_normal_connection_close) {
        return Ok(None);
    }
    let stream = stream.with_context(|| "Failed to accept unordered reliable QUIC stream")?;
    let mut reader = ReliableStreamReader::from(stream);

    let Some(message) = reader
        .recv(Delivery::ReliableUnordered)
        .await
        .with_context(|| "Failed to receive unordered reliable QUIC message")?
    else {
        eros::bail!("Unordered reliable QUIC stream ended before one complete message");
    };

    Ok(Some(message))
}

fn is_normal_connection_close(error: &compio::quic::ConnectionError) -> bool {
    match error {
        compio::quic::ConnectionError::ApplicationClosed(close) => {
            close.error_code == compio::quic::VarInt::from_u32(0)
        }
        compio::quic::ConnectionError::LocallyClosed => true,
        _ => false,
    }
}

impl From<compio::quic::RecvStream> for ReliableStreamReader {
    fn from(stream: compio::quic::RecvStream) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
            channel: None,
        }
    }
}

impl ReliableStreamReader {
    async fn recv(&mut self, delivery: Delivery) -> eros::Result<Option<TransportMessage>> {
        let Some(channel) = self.read_header().await? else {
            return Ok(None);
        };
        let payload_length = usize::from(u16::from_be_bytes([self.buffer[1], self.buffer[2]]));
        let frame_length = TLV_HEADER_SIZE + payload_length;

        if !self.fill_to(frame_length).await? {
            self.buffer.clear();
            return Ok(None);
        }

        let mut payload = self.buffer.split_to(frame_length).freeze();
        payload.advance(TLV_HEADER_SIZE);

        Ok(Some(TransportMessage {
            channel,
            delivery,
            payload,
        }))
    }

    async fn read_header(&mut self) -> eros::Result<Option<TransportChannel>> {
        if !self.fill_to(TLV_HEADER_SIZE).await? {
            self.buffer.clear();
            return Ok(None);
        }

        let channel = TransportChannel::from(self.buffer[0]);

        match self.channel {
            Some(expected) if expected != channel => {
                eros::bail!(
                    "Reliable QUIC stream changed channel from {expected:?} to {channel:?}"
                );
            }
            Some(_) => {}
            None => self.channel = Some(channel),
        }

        Ok(Some(channel))
    }

    async fn fill_to(&mut self, length: usize) -> eros::Result<bool> {
        while self.buffer.len() < length {
            let remaining = length - self.buffer.len();
            let chunk = self.stream.read_chunk(remaining, true).await;
            if chunk.as_ref().is_err_and(|error| {
                matches!(
                    error,
                    compio::quic::ReadError::ConnectionLost(error)
                        if is_normal_connection_close(error)
                )
            }) {
                return Ok(false);
            }
            let Some(chunk) = chunk.with_context(|| "Failed to read reliable QUIC stream")? else {
                return Ok(false);
            };

            self.buffer.extend_from_slice(&chunk.bytes);
        }

        Ok(true)
    }
}

fn encode_tlv(channel: TransportChannel, payload: Bytes) -> eros::Result<Bytes> {
    let payload_length = u16::try_from(payload.len())
        .with_context(|| "Failed to encode Transport TLV payload length")?;
    let mut tlv = BytesMut::with_capacity(TLV_HEADER_SIZE + payload.len());

    tlv.put_u8(channel.into());
    tlv.put_u16(payload_length);
    tlv.extend_from_slice(&payload);

    Ok(tlv.freeze())
}

fn encode_tlv_header(channel: TransportChannel, payload_length: usize) -> eros::Result<Bytes> {
    let payload_length = u16::try_from(payload_length)
        .with_context(|| "Failed to encode Transport TLV payload length")?;
    let mut header = BytesMut::with_capacity(TLV_HEADER_SIZE);

    header.put_u8(channel.into());
    header.put_u16(payload_length);

    Ok(header.freeze())
}

fn decode_tlv(mut tlv: Bytes, delivery: Delivery) -> eros::Result<TransportMessage> {
    if tlv.len() < TLV_HEADER_SIZE {
        eros::bail!("Transport TLV is missing its type or length");
    }

    let channel = TransportChannel::from(tlv.get_u8());
    let payload_length = usize::from(tlv.get_u16());

    if tlv.len() != payload_length {
        eros::bail!(
            "Transport TLV payload length is {}, expected {payload_length}",
            tlv.len(),
        );
    }

    Ok(TransportMessage {
        channel,
        delivery,
        payload: tlv,
    })
}

fn max_tlv_payload_size(max_datagram_size: Option<usize>) -> Option<usize> {
    max_datagram_size.map(|size| size.saturating_sub(TLV_HEADER_SIZE).min(u16::MAX as usize))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        net::{IpAddr, Ipv6Addr, SocketAddr},
    };

    use bytes::Bytes;

    use crate::{
        infra::{
            QuicEndpoint,
            transport::quic::{
                QuicTransport, ReceiveItem, is_normal_close_reason, max_tlv_payload_size,
                receive_next_message,
            },
            unsync_queue::UnsyncQueue,
        },
        kernel::transport::{
            Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
        },
    };

    #[test]
    fn reserves_the_quic_tlv_header_from_unreliable_payloads() {
        assert_eq!(max_tlv_payload_size(None), None);
        assert_eq!(max_tlv_payload_size(Some(2)), Some(0));
        assert_eq!(max_tlv_payload_size(Some(1200)), Some(1197));
        assert_eq!(max_tlv_payload_size(Some(usize::MAX)), Some(65535));
    }

    #[test]
    fn only_zero_application_close_and_local_close_are_normal() {
        assert!(is_normal_close_reason(Some(
            compio::quic::ConnectionError::LocallyClosed
        )));
        assert!(is_normal_close_reason(Some(
            compio::quic::ConnectionError::ApplicationClosed(compio::quic::ApplicationClose {
                error_code: compio::quic::VarInt::from_u32(0),
                reason: Bytes::from_static(b"normal"),
            })
        )));
        assert!(!is_normal_close_reason(Some(
            compio::quic::ConnectionError::ApplicationClosed(compio::quic::ApplicationClose {
                error_code: compio::quic::VarInt::from_u32(7),
                reason: Bytes::from_static(b"failure"),
            })
        )));
        assert!(!is_normal_close_reason(None));
    }

    #[test]
    fn transfers_one_received_batch_without_per_packet_task_handoffs() {
        let messages = UnsyncQueue::default();
        let consumed = UnsyncQueue::default();
        let first = TransportMessage {
            channel: TransportChannel::Video(crate::kernel::screen_manager::ScreenId(2)),
            delivery: Delivery::Unreliable,
            payload: Bytes::from_static(b"first"),
        };
        let second = TransportMessage {
            channel: TransportChannel::Video(crate::kernel::screen_manager::ScreenId(2)),
            delivery: Delivery::Unreliable,
            payload: Bytes::from_static(b"second"),
        };
        messages.push(ReceiveItem {
            result: Ok(Some(VecDeque::from([first.clone(), second.clone()]))),
            consumed: consumed.clone(),
        });
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");
        let mut pending = VecDeque::new();

        assert_eq!(
            runtime
                .block_on(receive_next_message(&mut pending, &messages))
                .expect("First batched message should be received"),
            Some(first)
        );
        assert!(consumed.try_pop().is_some());
        assert_eq!(
            runtime
                .block_on(receive_next_message(&mut pending, &messages))
                .expect("Second batched message should be received"),
            Some(second)
        );
    }

    #[test]
    fn concurrent_control_sends_are_serialized_by_the_writer() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let outgoing = QuicEndpoint::new_for_test()
                .await
                .expect("Outgoing QUIC endpoint should start");
            let incoming = QuicEndpoint::new_for_test()
                .await
                .expect("Incoming QUIC endpoint should start");
            let remote_address = SocketAddr::new(
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                incoming
                    .local_address()
                    .expect("Incoming endpoint should report its port")
                    .port(),
            );
            let (outgoing_connection, incoming_connection) = futures_util::join!(
                outgoing.connect(remote_address),
                incoming.accept_connection()
            );
            let outgoing_connection = outgoing_connection.expect("QUIC client should connect");
            let incoming_connection = incoming_connection
                .expect("QUIC server should accept without error")
                .expect("QUIC server should receive one connection");
            let (outgoing_transport, incoming_transport) = futures_util::join!(
                QuicTransport::open(outgoing_connection),
                QuicTransport::accept(incoming_connection)
            );
            let (send, _) = outgoing_transport
                .expect("Outgoing Transport should open")
                .split();
            let (_, mut recv) = incoming_transport
                .expect("Incoming Transport should open")
                .split();
            let first = TransportMessage {
                channel: TransportChannel::Control,
                delivery: Delivery::ReliableOrdered,
                payload: Bytes::from_static(b"first"),
            };
            let second = TransportMessage {
                channel: TransportChannel::Control,
                delivery: Delivery::ReliableOrdered,
                payload: Bytes::from_static(b"second"),
            };

            let (first_result, second_result) =
                futures_util::join!(send.send(first), send.send(second));
            first_result.expect("First Control send should complete");
            second_result.expect("Second Control send should queue and complete");

            assert_eq!(
                recv.recv()
                    .await
                    .expect("First Control receive should succeed")
                    .expect("First Control message should exist")
                    .payload,
                Bytes::from_static(b"first")
            );
            assert_eq!(
                recv.recv()
                    .await
                    .expect("Second Control receive should succeed")
                    .expect("Second Control message should exist")
                    .payload,
                Bytes::from_static(b"second")
            );
        });
    }

    #[test]
    fn normal_close_ends_the_remote_transport_receive() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let outgoing = QuicEndpoint::new_for_test()
                .await
                .expect("Outgoing QUIC endpoint should start");
            let incoming = QuicEndpoint::new_for_test()
                .await
                .expect("Incoming QUIC endpoint should start");
            let remote_address = SocketAddr::new(
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                incoming
                    .local_address()
                    .expect("Incoming endpoint should report its port")
                    .port(),
            );
            let (outgoing_connection, incoming_connection) = futures_util::join!(
                outgoing.connect(remote_address),
                incoming.accept_connection()
            );
            let outgoing_connection = outgoing_connection.expect("QUIC client should connect");
            let incoming_connection = incoming_connection
                .expect("QUIC server should accept without error")
                .expect("QUIC server should receive one connection");
            let (outgoing_transport, incoming_transport) = futures_util::join!(
                QuicTransport::open(outgoing_connection),
                QuicTransport::accept(incoming_connection)
            );
            let (send, _) = outgoing_transport
                .expect("Outgoing Transport should open")
                .split();
            let (_, mut recv) = incoming_transport
                .expect("Incoming Transport should open")
                .split();

            send.close().await;

            assert!(
                recv.recv()
                    .await
                    .expect("Normal QUIC close should not be a receive failure")
                    .is_none()
            );
        });
    }
}
