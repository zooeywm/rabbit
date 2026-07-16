use crate::infra::unsync_queue::UnsyncQueue;
use crate::kernel::transport::{
    Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use eros::Context;
use thin_cell::unsync::ThinCell;

const TLV_HEADER_SIZE: usize = size_of::<u8>() + size_of::<u16>();

type ReceiveResult = eros::Result<Option<TransportMessage>>;

pub(crate) struct QuicTransport {
    connection: compio::quic::Connection,
    control_send: compio::quic::SendStream,
    control_recv: compio::quic::RecvStream,
}

pub(crate) struct QuicTransportSend {
    connection: compio::quic::Connection,
    control_commands: UnsyncQueue<ControlWriteCommand>,
    control_sending: ThinCell<bool>,
    control_writer_closed: ThinCell<bool>,
    _control_writer: compio::runtime::JoinHandle<()>,
}

pub(crate) struct QuicTransportRecv {
    messages: UnsyncQueue<ReceiveResult>,
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
    _guard: ControlSendGuard,
}

struct ControlSendGuard {
    control_sending: ThinCell<bool>,
}

impl Drop for ControlSendGuard {
    fn drop(&mut self) {
        *self.control_sending.borrow() = false;
    }
}

impl QuicTransport {
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

async fn write_control_stream_preface(
    stream: &mut compio::quic::SendStream,
) -> eros::Result<()> {
    let mut preface = [Bytes::copy_from_slice(&[TransportChannel::Control.into()])];

    stream
        .write_all_chunks(&mut preface)
        .await
        .with_context(|| "Failed to write QUIC Control stream preface")?;

    Ok(())
}

async fn read_control_stream_preface(
    stream: &mut compio::quic::RecvStream,
) -> eros::Result<()> {
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
            control_sending: ThinCell::new(false),
            control_writer_closed: control_writer_closed.clone(),
            _control_writer: compio::runtime::spawn(run_control_writer(
                self.control_send,
                control_commands,
                control_writer_closed,
            )),
        };
        let recv = QuicTransportRecv {
            messages: messages.clone(),
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
        self.messages.pop()
    }
}

impl TransportSend for QuicTransportSend {
    fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>> {
        self.send_message(message)
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
            Delivery::Unreliable => self.send_unreliable(channel, payload),
        }
    }

    fn send_unreliable(&self, channel: TransportChannel, payload: Bytes) -> eros::Result<()> {
        let datagram =
            encode_tlv(channel, payload).with_context(|| "Failed to encode QUIC datagram")?;

        Ok(self
            .connection
            .send_datagram(datagram)
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

        let guard = self.acquire_control_send()?;
        let completion = UnsyncQueue::default();

        self.control_commands.push(ControlWriteCommand {
            channel,
            payload,
            completion: completion.clone(),
            _guard: guard,
        });

        completion.pop().await
    }

    fn acquire_control_send(&self) -> eros::Result<ControlSendGuard> {
        let mut control_sending = self.control_sending.borrow();

        if *control_sending {
            eros::bail!("A reliable ordered QUIC Control message is already being sent");
        }

        *control_sending = true;

        Ok(ControlSendGuard {
            control_sending: self.control_sending.clone(),
        })
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

        command.completion.push(result);

        if failed {
            *writer_closed.borrow() = true;
            return;
        }
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
    messages: UnsyncQueue<ReceiveResult>,
) {
    loop {
        let message = recv_reliable_ordered(&mut reader).await;
        let finished = !matches!(&message, Ok(Some(_)));

        messages.push(message);

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
    messages: UnsyncQueue<ReceiveResult>,
) {
    loop {
        let message = recv_reliable_unordered(&connection).await.map(Some);
        let failed = message.is_err();

        messages.push(message);

        if failed {
            return;
        }
    }
}

async fn receive_unreliable(
    connection: compio::quic::Connection,
    messages: UnsyncQueue<ReceiveResult>,
) {
    loop {
        match recv_unreliable(&connection).await {
            Ok(message) => {
                let channel = message.channel;

                match channel {
                    TransportChannel::Video(_) => {
                        messages.push_latest_by(Ok(Some(message)), |queued| {
                            matches!(
                                queued,
                                Ok(Some(TransportMessage {
                                    channel: queued_channel,
                                    delivery: Delivery::Unreliable,
                                    ..
                                })) if *queued_channel == channel
                            )
                        });
                    }
                    TransportChannel::Control => messages.push(Ok(Some(message))),
                }
            }
            Err(error) => {
                messages.push(Err(error));
                return;
            }
        }
    }
}

async fn recv_unreliable(connection: &compio::quic::Connection) -> eros::Result<TransportMessage> {
    let datagram = connection
        .recv_datagram()
        .await
        .with_context(|| "Failed to receive QUIC datagram")?;

    decode_tlv(datagram, Delivery::Unreliable).with_context(|| "Failed to decode QUIC datagram")
}

async fn recv_reliable_unordered(
    connection: &compio::quic::Connection,
) -> eros::Result<TransportMessage> {
    let stream = connection
        .accept_uni()
        .await
        .with_context(|| "Failed to accept unordered reliable QUIC stream")?;
    let mut reader = ReliableStreamReader::from(stream);

    let Some(message) = reader
        .recv(Delivery::ReliableUnordered)
        .await
        .with_context(|| "Failed to receive unordered reliable QUIC message")?
    else {
        eros::bail!("Unordered reliable QUIC stream ended before one complete message");
    };

    Ok(message)
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
            let Some(chunk) = self
                .stream
                .read_chunk(remaining, true)
                .await
                .with_context(|| "Failed to read reliable QUIC stream")?
            else {
                return Ok(false);
            };

            self.buffer.extend_from_slice(&chunk.bytes);
        }

        Ok(true)
    }
}

fn encode_tlv(channel: TransportChannel, payload: Bytes) -> eros::Result<Bytes> {
    let header = encode_tlv_header(channel, payload.len())?;
    let mut tlv = BytesMut::with_capacity(TLV_HEADER_SIZE + payload.len());

    tlv.extend_from_slice(&header);
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
