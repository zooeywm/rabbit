//! Connection handshake types: protocol version, capabilities, and responses.
//!
//! Wire encoding lives in `infra::connection_request`. This module is the
//! domain contract both peers must share.

use crate::kernel::{
    protocol::{MAX_VIDEO_SCREEN_ID, PROTOCOL_MAJOR, PROTOCOL_MINOR},
    screen_manager::ScreenId,
    video_encoder::VideoCodec,
};

/// Outbound or inbound connection request after the transport is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub requester_name: String,
    pub capabilities: PeerCapabilities,
}

/// Peer feature advertisement exchanged during the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerCapabilities {
    /// Maximum simultaneous video screens this peer can host or consume.
    pub max_screens: u8,
    /// Encoder profiles the peer can host (empty for pure controllers is OK).
    pub encoder_profiles: Vec<EncoderProfileTag>,
    /// Host accepts absolute pointer movements from a controller.
    pub absolute_pointer: bool,
    /// Host accepts Rabbit reliable keyboard, mouse-button, and relative-pointer input.
    pub reliable_input: bool,
}

impl Default for PeerCapabilities {
    fn default() -> Self {
        Self {
            max_screens: ScreenId::MAX.saturating_add(1),
            encoder_profiles: vec![EncoderProfileTag::H264Hardware],
            absolute_pointer: true,
            reliable_input: true,
        }
    }
}

impl PeerCapabilities {
    /// Builds host-oriented capabilities for the local process.
    pub fn local_host(screen_count: usize) -> Self {
        let max_screens = u8::try_from(screen_count)
            .unwrap_or(MAX_VIDEO_SCREEN_ID)
            .clamp(1, MAX_VIDEO_SCREEN_ID.saturating_add(1));
        Self {
            max_screens,
            encoder_profiles: vec![EncoderProfileTag::H264Hardware],
            absolute_pointer: true,
            reliable_input: true,
        }
    }
}

/// Wire tags for advertised encoder profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EncoderProfileTag {
    H264Hardware = 1,
    H264Software = 2,
}

impl EncoderProfileTag {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn codec(self) -> VideoCodec {
        match self {
            Self::H264Hardware | Self::H264Software => VideoCodec::H264,
        }
    }
}

impl TryFrom<u8> for EncoderProfileTag {
    type Error = UnknownEncoderProfile;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H264Hardware),
            2 => Ok(Self::H264Software),
            other => Err(UnknownEncoderProfile(other)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown encoder profile tag {0}")]
pub struct UnknownEncoderProfile(u8);

impl ConnectionRequest {
    /// Builds a request for this process using the current protocol constants.
    pub fn local(requester_name: String, capabilities: PeerCapabilities) -> Self {
        Self {
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            requester_name,
            capabilities,
        }
    }

    /// Peers must share a major version; minor is additive.
    pub fn is_protocol_compatible(&self) -> bool {
        self.protocol_major == PROTOCOL_MAJOR
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionResponse {
    Accepted = 0,
    Rejected = 1,
    SelfConnection = 2,
    /// Peer major version does not match this process.
    ProtocolMismatch = 3,
}

/// Full handshake reply after a connection request.
///
/// On accept, the host advertises its capabilities so the controller can size
/// stream requests without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionHandshakeReply {
    Accepted { host_capabilities: PeerCapabilities },
    Rejected,
    SelfConnection,
    ProtocolMismatch,
}

impl ConnectionHandshakeReply {
    pub fn status(&self) -> ConnectionResponse {
        match self {
            Self::Accepted { .. } => ConnectionResponse::Accepted,
            Self::Rejected => ConnectionResponse::Rejected,
            Self::SelfConnection => ConnectionResponse::SelfConnection,
            Self::ProtocolMismatch => ConnectionResponse::ProtocolMismatch,
        }
    }
}

impl From<ConnectionResponse> for u8 {
    fn from(response: ConnectionResponse) -> Self {
        response as Self
    }
}

impl TryFrom<u8> for ConnectionResponse {
    type Error = UnknownConnectionResponse;

    fn try_from(response: u8) -> Result<Self, Self::Error> {
        match response {
            0 => Ok(Self::Accepted),
            1 => Ok(Self::Rejected),
            2 => Ok(Self::SelfConnection),
            3 => Ok(Self::ProtocolMismatch),
            other => Err(UnknownConnectionResponse(other)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown connection response {0}")]
pub struct UnknownConnectionResponse(u8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_request_uses_current_protocol_constants() {
        let request = ConnectionRequest::local("peer".into(), PeerCapabilities::default());
        assert_eq!(request.protocol_major, PROTOCOL_MAJOR);
        assert_eq!(request.protocol_minor, PROTOCOL_MINOR);
        assert!(request.is_protocol_compatible());
    }

    #[test]
    fn incompatible_major_is_rejected_by_policy() {
        let mut request = ConnectionRequest::local("peer".into(), PeerCapabilities::default());
        request.protocol_major = PROTOCOL_MAJOR.wrapping_add(1);
        assert!(!request.is_protocol_compatible());
    }

    #[test]
    fn protocol_mismatch_response_round_trips() {
        let encoded = u8::from(ConnectionResponse::ProtocolMismatch);
        assert_eq!(
            ConnectionResponse::try_from(encoded).expect("decode"),
            ConnectionResponse::ProtocolMismatch
        );
    }

    #[test]
    fn encoder_profile_tags_round_trip() {
        for tag in [
            EncoderProfileTag::H264Hardware,
            EncoderProfileTag::H264Software,
        ] {
            assert_eq!(
                EncoderProfileTag::try_from(tag.as_u8()).expect("profile"),
                tag
            );
        }
    }
}
