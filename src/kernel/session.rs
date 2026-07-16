use crate::kernel::transport::{
    Transport, TransportMessage, TransportRecv, TransportSend,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRole {
    Controller,
    Host,
}

pub struct Session<T>
where
    T: Transport,
{
    role: SessionRole,
    send: T::SendHalf,
    recv: T::RecvHalf,
}

impl<T> Session<T>
where
    T: Transport,
{
    pub fn new(role: SessionRole, transport: T) -> Self {
        let (send, recv) = transport.split();
        Self { role, send, recv }
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn send(
        &self,
        message: TransportMessage,
    ) -> impl Future<Output = eros::Result<()>> {
        self.send.send(message)
    }

    pub fn recv(
        &mut self,
    ) -> impl Future<Output = eros::Result<Option<TransportMessage>>> {
        self.recv.recv()
    }
}
