mod quic;
mod tcp;

pub(crate) use quic::{QuicTransport, QuicTransportRecv, QuicTransportSend};
pub(crate) use tcp::{TcpTransport, TcpTransportRecv, TcpTransportSend};

use bytes::Bytes;

use crate::kernel::transport::{
    Transport, TransportChannel, TransportMessage, TransportRecv, TransportSend,
};

pub(crate) enum SessionTransport {
    Quic(QuicTransport),
    Tcp(TcpTransport),
}

pub(crate) enum SessionTransportSend {
    Quic(QuicTransportSend),
    Tcp(TcpTransportSend),
}

pub(crate) enum SessionTransportRecv {
    Quic(QuicTransportRecv),
    Tcp(TcpTransportRecv),
}

impl SessionTransport {
    pub(crate) fn remote_address(&self) -> std::net::SocketAddr {
        match self {
            Self::Quic(transport) => transport.remote_address(),
            Self::Tcp(transport) => transport.remote_address(),
        }
    }
}

impl Transport for SessionTransport {
    type SendHalf = SessionTransportSend;
    type RecvHalf = SessionTransportRecv;

    fn split(self) -> (Self::SendHalf, Self::RecvHalf) {
        match self {
            Self::Quic(transport) => {
                let (send, recv) = transport.split();
                (Self::SendHalf::Quic(send), Self::RecvHalf::Quic(recv))
            }
            Self::Tcp(transport) => {
                let (send, recv) = transport.split();
                (Self::SendHalf::Tcp(send), Self::RecvHalf::Tcp(recv))
            }
        }
    }
}

impl TransportSend for SessionTransportSend {
    fn max_unreliable_payload_size(&self) -> Option<usize> {
        match self {
            Self::Quic(send) => send.max_unreliable_payload_size(),
            Self::Tcp(send) => send.max_unreliable_payload_size(),
        }
    }

    fn is_closed_normally(&self) -> bool {
        match self {
            Self::Quic(send) => send.is_closed_normally(),
            Self::Tcp(send) => send.is_closed_normally(),
        }
    }

    async fn send_unreliable(&self, channel: TransportChannel, payload: Bytes) -> eros::Result<()> {
        match self {
            Self::Quic(send) => send.send_unreliable(channel, payload).await,
            Self::Tcp(send) => send.send_unreliable(channel, payload).await,
        }
    }

    async fn send(&self, message: TransportMessage) -> eros::Result<()> {
        match self {
            Self::Quic(send) => send.send(message).await,
            Self::Tcp(send) => send.send(message).await,
        }
    }

    async fn close(&self) {
        match self {
            Self::Quic(send) => send.close().await,
            Self::Tcp(send) => send.close().await,
        }
    }
}

impl TransportRecv for SessionTransportRecv {
    async fn recv(&mut self) -> eros::Result<Option<TransportMessage>> {
        match self {
            Self::Quic(recv) => recv.recv().await,
            Self::Tcp(recv) => recv.recv().await,
        }
    }
}

// Focused tests: cargo test infra::transport:: --lib
