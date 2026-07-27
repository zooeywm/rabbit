//! Session transport ports: channels, delivery, and byte-oriented messaging.
//!
//! Channel numbering is fixed by [`crate::kernel::protocol`] so control and
//! video cannot silently renumber across peers.

use crate::kernel::{protocol::CONTROL_CHANNEL_ID, screen_manager::ScreenId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportChannel {
    Control,
    Video(ScreenId),
}

impl From<TransportChannel> for u8 {
    fn from(channel: TransportChannel) -> Self {
        match channel {
            TransportChannel::Control => CONTROL_CHANNEL_ID,
            TransportChannel::Video(id) => u8::from(id).saturating_add(1),
        }
    }
}

impl From<u8> for TransportChannel {
    fn from(id: u8) -> Self {
        if id == CONTROL_CHANNEL_ID {
            Self::Control
        } else {
            Self::Video(ScreenId(id - 1))
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

    fn is_closed_normally(&self) -> bool {
        false
    }

    fn send_unreliable(
        &self,
        channel: TransportChannel,
        payload: bytes::Bytes,
    ) -> impl Future<Output = eros::Result<()>>;

    fn send(&self, message: TransportMessage) -> impl Future<Output = eros::Result<()>>;

    fn close(&self) -> impl Future<Output = ()>;
}

pub trait TransportRecv {
    fn recv(&mut self) -> impl Future<Output = eros::Result<Option<TransportMessage>>>;
}
