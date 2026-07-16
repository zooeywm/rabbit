#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub requester_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionResponse {
    Accepted,
    Rejected,
}
