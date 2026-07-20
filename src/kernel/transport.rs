use crate::kernel::screen_manager::ScreenId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportChannel {
    Control,
    Video(ScreenId),
}

impl From<TransportChannel> for u8 {
    fn from(channel: TransportChannel) -> Self {
        match channel {
            TransportChannel::Control => 0,
            TransportChannel::Video(id) => u8::from(id) + 1,
        }
    }
}

impl From<u8> for TransportChannel {
    fn from(id: u8) -> Self {
        match id {
            0 => Self::Control,
            id => Self::Video(ScreenId(id - 1)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delivery {
    ReliableOrdered,
    ReliableUnordered,
    Unreliable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportMessage {
    pub channel: TransportChannel,
    pub delivery: Delivery,
    pub payload: bytes::Bytes,
}

pub trait Transport {
    type SendHalf: TransportSend;
    type RecvHalf: TransportRecv;

    fn split(self) -> (Self::SendHalf, Self::RecvHalf);
}

pub trait TransportSend {
    fn max_unreliable_payload_size(&self) -> Option<usize>;

    fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>>;
}

pub trait TransportRecv {
    fn recv(&mut self) -> impl Future<Output = eros::Result<Option<TransportMessage>>>;
}
