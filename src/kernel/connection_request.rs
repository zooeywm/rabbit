#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub requester_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionResponse {
    Accepted,
    Rejected,
    SelfConnection,
}

impl From<ConnectionResponse> for u8 {
    fn from(response: ConnectionResponse) -> Self {
        response as Self
    }
}

impl TryFrom<u8> for ConnectionResponse {
    type Error = UnknownConnectionResponse;

    fn try_from(response: u8) -> Result<Self, Self::Error> {
        if response == Self::Accepted as u8 {
            Ok(Self::Accepted)
        } else if response == Self::Rejected as u8 {
            Ok(Self::Rejected)
        } else if response == Self::SelfConnection as u8 {
            Ok(Self::SelfConnection)
        } else {
            Err(UnknownConnectionResponse(response))
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown connection response {0}")]
pub struct UnknownConnectionResponse(u8);
