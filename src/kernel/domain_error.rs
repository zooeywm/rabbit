//! Domain error taxonomy for protocol and session policy.
//!
//! Infrastructure maps I/O failures into richer contexts; domain code should
//! prefer these kinds so UI, headless, and logs can classify failures without
//! parsing free-form strings.

use std::fmt;

/// Stable, user-facing domain failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DomainErrorKind {
    /// Wire or version incompatibility.
    Protocol,
    /// Peer or local capability cannot satisfy the request.
    Capability,
    /// Illegal session / stream lifecycle operation.
    SessionState,
    /// Referenced screen, session, or stream does not exist.
    NotFound,
    /// Internal invariant broken (should be rare).
    Internal,
}

impl DomainErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Protocol => "protocol",
            Self::Capability => "capability",
            Self::SessionState => "session_state",
            Self::NotFound => "not_found",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for DomainErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured domain error with stable kind + message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct DomainError {
    pub kind: DomainErrorKind,
    pub message: String,
}

impl DomainError {
    pub fn new(kind: DomainErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::Protocol, message)
    }

    pub fn capability(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::Capability, message)
    }

    pub fn session_state(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::SessionState, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::Internal, message)
    }

    pub fn into_union(self) -> eros::ErrorUnion {
        let message = self.to_string();
        eros::error!("{}", message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_have_stable_string_codes() {
        assert_eq!(DomainErrorKind::Capability.as_str(), "capability");
        assert_eq!(
            DomainError::session_state("draining").to_string(),
            "session_state: draining"
        );
    }
}
