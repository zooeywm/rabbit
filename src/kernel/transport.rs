#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportChannelId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delivery {
    ReliableOrdered,
    BestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportMessage {
    pub channel_id: TransportChannelId,
    pub delivery: Delivery,
    pub payload: Vec<u8>,
}

pub trait Transport {
    type SendHalf: TransportSend;
    type RecvHalf: TransportRecv;

    fn split(self) -> (Self::SendHalf, Self::RecvHalf);
}

#[allow(async_fn_in_trait)]
pub trait TransportSend {
    async fn send(&mut self, message: TransportMessage) -> eros::Result<()>;
}

#[allow(async_fn_in_trait)]
pub trait TransportRecv {
    async fn recv(&mut self) -> eros::Result<Option<TransportMessage>>;
}
