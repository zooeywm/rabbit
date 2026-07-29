//! Wire-protocol identity and evolution policy.
//!
//! Rabbit peers speak a single application protocol layered over the selected
//! transport (`QUIC` or `TCP`). This module is the **single source of truth**
//! for protocol identity so adapters and session handshakes cannot drift.
//!
//! # Versioning policy
//!
//! - `PROTOCOL_MAJOR` increments for wire-incompatible changes (new mandatory
//!   handshake fields, removed control tags, channel renumbering).
//! - `PROTOCOL_MINOR` increments for backward-compatible additions (new optional
//!   control messages, optional capabilities).
//! - Peers that share a major version must interoperate. A higher minor version
//!   must tolerate a lower minor peer by ignoring unknown optional features.
//!
//! Handshake carries major/minor plus capabilities (see
//! [`crate::kernel::connection_request`]). Stream admission consults
//! [`crate::kernel::capability`].

/// Incompatible protocol generation.
pub const PROTOCOL_MAJOR: u16 = 4;

/// Compatible extension level within [`PROTOCOL_MAJOR`].
pub const PROTOCOL_MINOR: u16 = 1;

/// Human-readable protocol identity for logs and diagnostics.
pub const PROTOCOL_NAME: &str = "rabbit-session";

/// Returns the dotted protocol version string (`"{major}.{minor}"`).
pub const fn protocol_version_string() -> &'static str {
    // Keep this a const so it can appear in static diagnostics without format!.
    // Update both the constants above and this string together.
    "4.1"
}

/// Control-plane channel id on the session transport.
///
/// Must stay aligned with [`crate::kernel::transport::TransportChannel::Control`].
pub const CONTROL_CHANNEL_ID: u8 = 0;

/// Maximum video screen id that can be mapped onto a transport channel.
///
/// Channel `0` is reserved for control; video channels use `screen_id + 1`, so
/// screen ids occupy `0..=254`.
pub const MAX_VIDEO_SCREEN_ID: u8 = u8::MAX - 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{screen_manager::ScreenId, transport::TransportChannel};

    #[test]
    fn protocol_version_string_matches_numeric_constants() {
        assert_eq!(
            protocol_version_string(),
            const_format_args(PROTOCOL_MAJOR, PROTOCOL_MINOR)
        );
    }

    fn const_format_args(major: u16, minor: u16) -> String {
        format!("{major}.{minor}")
    }

    #[test]
    fn control_channel_and_screen_id_budget_match_transport_mapping() {
        assert_eq!(u8::from(TransportChannel::Control), CONTROL_CHANNEL_ID);
        assert_eq!(ScreenId::MAX, MAX_VIDEO_SCREEN_ID);
        assert_eq!(
            u8::from(TransportChannel::Video(ScreenId(MAX_VIDEO_SCREEN_ID))),
            u8::MAX
        );
    }

    #[test]
    fn protocol_name_is_stable_for_diagnostics() {
        assert_eq!(PROTOCOL_NAME, "rabbit-session");
    }
}
