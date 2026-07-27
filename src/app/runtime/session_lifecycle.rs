//! Exhaustive session phase transition table.
//!
//! Illegal transitions return [`DomainError`] instead of silently no-oping, so
//! callers and tests share one enforceable state machine.

use crate::kernel::domain_error::DomainError;

/// Lifecycle phase of a registered peer session.
///
/// ```text
/// Joining ──Activate──► Active ──BeginDrain──► Draining
///    │                    │                      │
///    └────BeginDrain──────┴──────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionPhase {
    Joining,
    Active,
    Draining,
}

/// Events that may advance [`SessionPhase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionPhaseEvent {
    Activate,
    BeginDrain,
}

impl SessionPhase {
    pub const ALL: [Self; 3] = [Self::Joining, Self::Active, Self::Draining];

    pub const fn admits_new_streams(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Applies one event. Returns the next phase or a structured domain error.
    pub fn transition(self, event: SessionPhaseEvent) -> Result<Self, DomainError> {
        use SessionPhase as P;
        use SessionPhaseEvent as E;

        match (self, event) {
            (P::Joining, E::Activate) => Ok(P::Active),
            (P::Joining, E::BeginDrain) => Ok(P::Draining),
            (P::Active, E::BeginDrain) => Ok(P::Draining),
            (P::Active, E::Activate) => Err(DomainError::session_state(
                "cannot Activate an already Active session",
            )),
            (P::Draining, E::Activate) => Err(DomainError::session_state(
                "cannot Activate a Draining session",
            )),
            (P::Draining, E::BeginDrain) => {
                Err(DomainError::session_state("session is already Draining"))
            }
        }
    }
}

/// Every (phase, event) pair is classified as either allowed or rejected.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table_is_exhaustive_and_stable() {
        let mut allowed = 0u32;
        let mut rejected = 0u32;

        for phase in SessionPhase::ALL {
            for event in [SessionPhaseEvent::Activate, SessionPhaseEvent::BeginDrain] {
                match phase.transition(event) {
                    Ok(next) => {
                        allowed += 1;
                        assert_ne!(
                            (phase, event),
                            (SessionPhase::Draining, SessionPhaseEvent::Activate),
                            "draining must never activate"
                        );
                        // Progress is monotonic toward drain except activate.
                        if event == SessionPhaseEvent::BeginDrain {
                            assert_eq!(next, SessionPhase::Draining);
                        }
                    }
                    Err(error) => {
                        rejected += 1;
                        assert_eq!(
                            error.kind,
                            crate::kernel::domain_error::DomainErrorKind::SessionState
                        );
                    }
                }
            }
        }

        // 3 phases × 2 events = 6; 3 legal paths (join→active, join→drain, active→drain).
        assert_eq!(allowed, 3);
        assert_eq!(rejected, 3);
    }

    #[test]
    fn only_active_admits_streams() {
        assert!(!SessionPhase::Joining.admits_new_streams());
        assert!(SessionPhase::Active.admits_new_streams());
        assert!(!SessionPhase::Draining.admits_new_streams());
    }
}
