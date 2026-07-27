//! Capability negotiation policy (pure, transport-agnostic).
//!
//! Handshake advertises [`PeerCapabilities`]; stream setup must consult them
//! before starting host pipelines so peers never promise work they cannot do.

use crate::kernel::{
    connection_request::{EncoderProfileTag, PeerCapabilities},
    domain_error::DomainError,
    screen_configuration::SetScreenStreams,
};

/// Local host requirements for the current product encode path.
pub fn local_host_encode_requirements() -> &'static [EncoderProfileTag] {
    &[EncoderProfileTag::H264Hardware]
}

/// Ensures the local host can satisfy its own encode path.
pub fn assert_local_can_host(local: &PeerCapabilities) -> Result<(), DomainError> {
    for required in local_host_encode_requirements() {
        if !local.encoder_profiles.contains(required) {
            return Err(DomainError::capability(format!(
                "local host lacks required encoder profile {required:?}"
            )));
        }
    }
    Ok(())
}

/// Validates a controller stream request against both peers' capabilities.
///
/// Rules:
/// - session must already admit streams (caller passes `admits_streams`)
/// - stream count must not exceed the **peer's** `max_screens` (consumer budget)
/// - stream count must not exceed the **local** `max_screens` (host budget)
/// - local host must advertise a usable encode profile
pub fn validate_set_screen_streams(
    request: &SetScreenStreams,
    local: &PeerCapabilities,
    peer: &PeerCapabilities,
    admits_streams: bool,
) -> Result<(), DomainError> {
    if !admits_streams {
        return Err(DomainError::session_state(
            "session does not admit new screen streams in its current phase",
        ));
    }

    assert_local_can_host(local)?;

    let requested = request.desired_streams.len();
    let requested_u8 = u8::try_from(requested).map_err(|_| {
        DomainError::capability(format!(
            "requested {requested} streams exceeds u8 screen budget"
        ))
    })?;

    if requested_u8 > local.max_screens {
        return Err(DomainError::capability(format!(
            "requested {requested} streams exceeds local host max_screens {}",
            local.max_screens
        )));
    }
    if requested_u8 > peer.max_screens {
        return Err(DomainError::capability(format!(
            "requested {requested} streams exceeds peer max_screens {}",
            peer.max_screens
        )));
    }

    // Controllers may omit encoder profiles; hosts must still encode.
    // If the peer advertised profiles, require intersection with local host encode path.
    if !peer.encoder_profiles.is_empty() {
        let overlap = local_host_encode_requirements()
            .iter()
            .any(|required| peer.encoder_profiles.contains(required));
        if !overlap {
            return Err(DomainError::capability(
                "peer encoder profiles do not intersect local host encode requirements",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::{
            RemoteDisplayMode, ScreenStreamRequest, ScreenStreamRequestId, SetScreenStreams,
        },
        screen_manager::ScreenId,
    };

    fn request_with_n_streams(n: usize) -> SetScreenStreams {
        SetScreenStreams {
            request_id: ScreenStreamRequestId(1),
            desired_streams: (0..n)
                .map(|i| ScreenStreamRequest {
                    screen_id: ScreenId(i as u8),
                    remote_display: RemoteDisplayMode::Preserve,
                    frame_size: PixelSize {
                        width: 1920,
                        height: 1080,
                    },
                    frame_rate: FrameRate::new(60, 1).expect("fps"),
                })
                .collect(),
        }
    }

    #[test]
    fn rejects_when_session_does_not_admit_streams() {
        let err = validate_set_screen_streams(
            &request_with_n_streams(1),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            false,
        )
        .expect_err("must reject");
        assert_eq!(err.kind, crate::kernel::domain_error::DomainErrorKind::SessionState);
    }

    #[test]
    fn rejects_when_peer_max_screens_exceeded() {
        let peer = PeerCapabilities {
            max_screens: 1,
            encoder_profiles: vec![EncoderProfileTag::H264Hardware],
        };
        let err = validate_set_screen_streams(
            &request_with_n_streams(2),
            &PeerCapabilities::default(),
            &peer,
            true,
        )
        .expect_err("must reject");
        assert_eq!(err.kind, crate::kernel::domain_error::DomainErrorKind::Capability);
    }

    #[test]
    fn accepts_compatible_single_stream() {
        validate_set_screen_streams(
            &request_with_n_streams(1),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
        )
        .expect("compatible request");
    }
}
