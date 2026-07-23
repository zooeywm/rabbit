use bytes::{BufMut, Bytes, BytesMut};
use compio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use eros::Context;
use thin_cell::unsync::ThinCell;

use crate::{
    infra::unsync_queue::UnsyncQueue,
    kernel::transport::{
        Delivery, Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
    },
};

const FRAME_HEADER_SIZE: usize = size_of::<u8>() + size_of::<u8>() + size_of::<u16>();
const MAX_UNRELIABLE_PAYLOAD_SIZE: usize = 1_200;

pub(crate) struct TcpTransport {
    stream: compio::net::TcpStream,
    remote_address: std::net::SocketAddr,
}

pub(crate) struct TcpTransportSend {
    commands: UnsyncQueue<WriteCommand>,
    state: ThinCell<TransportState>,
    _writer: compio::runtime::JoinHandle<()>,
}

pub(crate) struct TcpTransportRecv {
    stream: compio::net::TcpStream,
    state: ThinCell<TransportState>,
}

struct WriteCommand {
    operation: WriteOperation,
    completion: UnsyncQueue<eros::Result<()>>,
}

enum WriteOperation {
    Frame(Bytes),
    Close,
}

#[derive(Clone, Copy)]
struct TransportState {
    writer_available: bool,
    closed_normally: bool,
}

impl TcpTransport {
    pub(crate) fn new(stream: compio::net::TcpStream) -> eros::Result<Self> {
        let remote_address = stream
            .peer_addr()
            .with_context(|| "Failed to read TCP transport peer address")?;

        Ok(Self {
            stream,
            remote_address,
        })
    }

    pub(crate) fn remote_address(&self) -> std::net::SocketAddr {
        self.remote_address
    }
}

impl Transport for TcpTransport {
    type SendHalf = TcpTransportSend;
    type RecvHalf = TcpTransportRecv;

    fn split(self) -> (Self::SendHalf, Self::RecvHalf) {
        let (read_stream, write_stream) = self.stream.into_split();
        let commands = UnsyncQueue::default();
        let state = ThinCell::new(TransportState {
            writer_available: true,
            closed_normally: false,
        });

        (
            TcpTransportSend {
                commands: commands.clone(),
                state: state.clone(),
                _writer: compio::runtime::spawn(run_writer(write_stream, commands, state.clone())),
            },
            TcpTransportRecv {
                stream: read_stream,
                state,
            },
        )
    }
}

impl TransportSend for TcpTransportSend {
    fn max_unreliable_payload_size(&self) -> Option<usize> {
        Some(MAX_UNRELIABLE_PAYLOAD_SIZE)
    }

    fn is_closed_normally(&self) -> bool {
        self.state.borrow().closed_normally
    }

    fn send_unreliable(
        &self,
        channel: TransportChannel,
        payload: Bytes,
    ) -> impl Future<Output = eros::Result<()>> {
        self.send_message(TransportMessage {
            channel,
            delivery: Delivery::Unreliable,
            payload,
        })
    }

    fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>> {
        self.send_message(message)
    }

    async fn close(&self) {
        if !self.state.borrow().writer_available {
            return;
        }

        let completion = UnsyncQueue::default();
        self.commands.push(WriteCommand {
            operation: WriteOperation::Close,
            completion: completion.clone(),
        });
        let _ = completion.pop().await;
    }
}

impl TcpTransportSend {
    async fn send_message(&self, message: TransportMessage) -> eros::Result<()> {
        if !self.state.borrow().writer_available {
            eros::bail!("TCP transport writer is unavailable");
        }

        let frame =
            encode_frame(message).with_context(|| "Failed to encode TCP transport frame")?;
        let completion = UnsyncQueue::default();
        self.commands.push(WriteCommand {
            operation: WriteOperation::Frame(frame),
            completion: completion.clone(),
        });

        completion.pop().await
    }
}

impl TransportRecv for TcpTransportRecv {
    fn recv(&mut self) -> impl Future<Output = eros::Result<Option<TransportMessage>>> {
        recv_frame(&mut self.stream, &self.state)
    }
}

async fn run_writer(
    mut stream: compio::net::TcpStream,
    commands: UnsyncQueue<WriteCommand>,
    state: ThinCell<TransportState>,
) {
    loop {
        let command = commands.pop().await;
        let (result, finished) = match command.operation {
            WriteOperation::Frame(frame) => (write_frame(&mut stream, frame).await, false),
            WriteOperation::Close => {
                {
                    let mut state = state.borrow();
                    state.closed_normally = true;
                }
                (shutdown_writer(&mut stream).await, true)
            }
        };
        let failed = result.is_err();
        command.completion.push(result);

        if failed || finished {
            state.borrow().writer_available = false;
            while let Some(command) = commands.try_pop() {
                command.completion.push(Err(eros::error!(
                    "TCP transport writer stopped before this operation"
                )));
            }
            return;
        }
    }
}

async fn write_frame(stream: &mut compio::net::TcpStream, frame: Bytes) -> eros::Result<()> {
    Ok(stream
        .write_all(frame)
        .await
        .0
        .with_context(|| "Failed to write TCP transport frame")?)
}

async fn shutdown_writer(stream: &mut compio::net::TcpStream) -> eros::Result<()> {
    Ok(stream
        .shutdown()
        .await
        .with_context(|| "Failed to shut down TCP transport writer")?)
}

async fn recv_frame(
    stream: &mut compio::net::TcpStream,
    state: &ThinCell<TransportState>,
) -> eros::Result<Option<TransportMessage>> {
    let header_result = stream
        .read_exact(Vec::with_capacity(FRAME_HEADER_SIZE))
        .await;
    let header = match header_result.0 {
        Ok(()) => header_result.1,
        Err(error)
            if error.kind() == std::io::ErrorKind::UnexpectedEof && header_result.1.is_empty() =>
        {
            state.borrow().closed_normally = true;
            return Ok(None);
        }
        Err(error) => {
            return Ok(Err(error).with_context(|| "Failed to read TCP transport frame header")?);
        }
    };
    let delivery = decode_delivery(header[0])?;
    let channel = TransportChannel::from(header[1]);
    let payload_length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let payload_result = stream.read_exact(Vec::with_capacity(payload_length)).await;
    let payload = match payload_result.0 {
        Ok(()) => payload_result.1,
        Err(error) => {
            return Ok(Err(error).with_context(|| "Failed to read TCP transport frame payload")?);
        }
    };

    Ok(Some(TransportMessage {
        channel,
        delivery,
        payload: Bytes::from(payload),
    }))
}

fn encode_frame(message: TransportMessage) -> eros::Result<Bytes> {
    let payload_length = u16::try_from(message.payload.len())
        .with_context(|| "Failed to encode TCP transport payload length")?;
    let mut frame = BytesMut::with_capacity(FRAME_HEADER_SIZE + message.payload.len());

    frame.put_u8(encode_delivery(message.delivery));
    frame.put_u8(message.channel.into());
    frame.put_u16(payload_length);
    frame.extend_from_slice(&message.payload);

    Ok(frame.freeze())
}

const fn encode_delivery(delivery: Delivery) -> u8 {
    match delivery {
        Delivery::ReliableOrdered => 0,
        Delivery::ReliableUnordered => 1,
        Delivery::Unreliable => 2,
    }
}

fn decode_delivery(delivery: u8) -> eros::Result<Delivery> {
    match delivery {
        0 => Ok(Delivery::ReliableOrdered),
        1 => Ok(Delivery::ReliableUnordered),
        2 => Ok(Delivery::Unreliable),
        _ => eros::bail!("TCP transport frame has unknown delivery {}", delivery),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{
        infra::transport::tcp::TcpTransport,
        kernel::{
            screen_manager::ScreenId,
            transport::{
                Delivery, Transport, TransportChannel, TransportMessage, TransportRecv,
                TransportSend,
            },
        },
    };

    #[test]
    fn transports_all_delivery_markers_over_one_tcp_stream() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let listener = compio::net::TcpListener::bind("[::1]:0")
                .await
                .expect("Test TCP listener should bind");
            let address = listener
                .local_addr()
                .expect("Test TCP listener address should be available");
            let outgoing = compio::net::TcpStream::connect(address);
            let incoming = listener.accept();
            let (outgoing, (incoming, _)) = futures_util::try_join!(outgoing, incoming)
                .expect("Test TCP connection should establish");
            let (send, _) = TcpTransport::new(outgoing)
                .expect("Outgoing TCP transport should initialize")
                .split();
            let (_, mut recv) = TcpTransport::new(incoming)
                .expect("Incoming TCP transport should initialize")
                .split();
            let messages = [
                TransportMessage {
                    channel: TransportChannel::Control,
                    delivery: Delivery::ReliableOrdered,
                    payload: Bytes::from_static(b"control"),
                },
                TransportMessage {
                    channel: TransportChannel::Video(ScreenId(0)),
                    delivery: Delivery::ReliableUnordered,
                    payload: Bytes::from_static(b"audio-later"),
                },
                TransportMessage {
                    channel: TransportChannel::Video(ScreenId(1)),
                    delivery: Delivery::Unreliable,
                    payload: Bytes::from_static(b"video"),
                },
            ];

            for message in messages.clone() {
                send.send(message).await.expect("TCP message should send");
            }
            for expected in messages {
                assert_eq!(
                    recv.recv().await.expect("TCP message should receive"),
                    Some(expected)
                );
            }
            assert_eq!(send.max_unreliable_payload_size(), Some(1_200));
        });
    }
}

// Focused test: cargo test infra::transport::tcp::tests:: --lib
