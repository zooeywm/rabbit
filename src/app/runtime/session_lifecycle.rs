//! Exhaustive session phase transition table, timeouts, and reconnect policy.
//!
//! Illegal transitions return [`DomainError`] instead of silently no-oping, so
//! callers and tests share one enforceable state machine.

use std::time::Duration;

use crate::kernel::domain_error::DomainError;

/// Lifecycle phase of a registered peer session.
///
/// ```text
/// Joining ──Activate──► Active ──BeginDrain──► Draining
///    │                    │                      │
///    ├────BeginDrain──────┤                      │
///    └────JoinTimedOut────┘                      │
///                          IdleTimedOut ─────────┘
///                          DrainTimedOut (force-remove signal; stays Draining)
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
    /// Handshake / activation deadline exceeded while still Joining.
    JoinTimedOut,
    /// Active session idle (no streams) deadline exceeded.
    IdleTimedOut,
    /// Draining deadline exceeded — shells should remove the session.
    DrainTimedOut,
}

impl SessionPhase {
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Joining, Self::Active, Self::Draining];

    pub const fn admits_new_streams(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Phase is still registered but no longer admits product work.
    pub const fn is_draining(self) -> bool {
        matches!(self, Self::Draining)
    }

    /// Applies one event. Returns the next phase or a structured domain error.
    pub fn transition(self, event: SessionPhaseEvent) -> Result<Self, DomainError> {
        use SessionPhase as P;
        use SessionPhaseEvent as E;

        match (self, event) {
            (P::Joining, E::Activate) => Ok(P::Active),
            (P::Joining, E::BeginDrain | E::JoinTimedOut) => Ok(P::Draining),
            (P::Joining, E::IdleTimedOut) => Err(DomainError::session_state(
                "cannot idle-timeout a Joining session (use JoinTimedOut)",
            )),
            (P::Joining, E::DrainTimedOut) => Err(DomainError::session_state(
                "cannot drain-timeout a Joining session",
            )),

            (P::Active, E::BeginDrain | E::IdleTimedOut) => Ok(P::Draining),
            (P::Active, E::Activate) => Err(DomainError::session_state(
                "cannot Activate an already Active session",
            )),
            (P::Active, E::JoinTimedOut) => Err(DomainError::session_state(
                "cannot join-timeout an Active session",
            )),
            (P::Active, E::DrainTimedOut) => Err(DomainError::session_state(
                "cannot drain-timeout an Active session (use BeginDrain or IdleTimedOut)",
            )),

            (P::Draining, E::DrainTimedOut) => Ok(P::Draining),
            (P::Draining, E::Activate) => Err(DomainError::session_state(
                "cannot Activate a Draining session",
            )),
            (P::Draining, E::BeginDrain | E::JoinTimedOut | E::IdleTimedOut) => {
                Err(DomainError::session_state("session is already Draining"))
            }
        }
    }
}

/// Default wall-clock budgets for phase-local timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTimeoutPolicy {
    /// Max time to remain in [`SessionPhase::Joining`] before [`SessionPhaseEvent::JoinTimedOut`].
    pub join: Duration,
    /// Max time in [`SessionPhase::Active`] with zero streams before idle drain.
    pub idle_active: Duration,
    /// Max time in [`SessionPhase::Draining`] before force-remove.
    pub drain: Duration,
}

impl Default for SessionTimeoutPolicy {
    fn default() -> Self {
        Self {
            join: Duration::from_secs(30),
            idle_active: Duration::from_secs(0), // 0 = disabled
            drain: Duration::from_secs(15),
        }
    }
}

/// Policy outcome when a phase budget is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTimeoutAction {
    /// Transition with this event (may move Joining/Active → Draining).
    Transition(SessionPhaseEvent),
}

/// Pure timeout classifier: given phase, elapsed time in that phase, stream count,
/// and policy, returns the action a shell should take (if any).
pub fn evaluate_session_timeout(
    phase: SessionPhase,
    phase_elapsed: Duration,
    active_stream_count: usize,
    policy: &SessionTimeoutPolicy,
) -> Option<SessionTimeoutAction> {
    match phase {
        SessionPhase::Joining if phase_elapsed >= policy.join && !policy.join.is_zero() => Some(
            SessionTimeoutAction::Transition(SessionPhaseEvent::JoinTimedOut),
        ),
        SessionPhase::Active
            if active_stream_count == 0
                && !policy.idle_active.is_zero()
                && phase_elapsed >= policy.idle_active =>
        {
            Some(SessionTimeoutAction::Transition(
                SessionPhaseEvent::IdleTimedOut,
            ))
        }
        SessionPhase::Draining if phase_elapsed >= policy.drain && !policy.drain.is_zero() => Some(
            SessionTimeoutAction::Transition(SessionPhaseEvent::DrainTimedOut),
        ),
        _ => None,
    }
}

/// Whether a new session may supersede a previous registration for the same peer key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectEligibility {
    Allowed,
    Denied,
}

/// Reconnect replaces identity only when no live (Joining/Active) session remains.
pub fn evaluate_reconnect(existing_phase: Option<SessionPhase>) -> ReconnectEligibility {
    match existing_phase {
        None => ReconnectEligibility::Allowed,
        Some(phase) => {
            if phase.is_draining() {
                ReconnectEligibility::Allowed
            } else {
                ReconnectEligibility::Denied
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
        let events = [
            SessionPhaseEvent::Activate,
            SessionPhaseEvent::BeginDrain,
            SessionPhaseEvent::JoinTimedOut,
            SessionPhaseEvent::IdleTimedOut,
            SessionPhaseEvent::DrainTimedOut,
        ];

        for phase in SessionPhase::ALL {
            for event in events {
                match phase.transition(event) {
                    Ok(next) => {
                        allowed += 1;
                        assert_ne!(
                            (phase, event),
                            (SessionPhase::Draining, SessionPhaseEvent::Activate),
                            "draining must never activate"
                        );
                        if matches!(
                            event,
                            SessionPhaseEvent::BeginDrain
                                | SessionPhaseEvent::JoinTimedOut
                                | SessionPhaseEvent::IdleTimedOut
                        ) {
                            assert_eq!(next, SessionPhase::Draining);
                        }
                        if event == SessionPhaseEvent::DrainTimedOut {
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

        // 3 phases × 5 events = 15; legal:
        // join: Activate, BeginDrain, JoinTimedOut (3)
        // active: BeginDrain, IdleTimedOut (2)
        // draining: DrainTimedOut (1)
        assert_eq!(allowed, 6);
        assert_eq!(rejected, 9);
    }

    #[test]
    fn only_active_admits_streams() {
        assert!(!SessionPhase::Joining.admits_new_streams());
        assert!(SessionPhase::Active.admits_new_streams());
        assert!(!SessionPhase::Draining.admits_new_streams());
    }

    #[test]
    fn join_timeout_drains_joining_only() {
        assert_eq!(
            SessionPhase::Joining
                .transition(SessionPhaseEvent::JoinTimedOut)
                .unwrap(),
            SessionPhase::Draining
        );
        assert!(
            SessionPhase::Active
                .transition(SessionPhaseEvent::JoinTimedOut)
                .is_err()
        );
    }

    #[test]
    fn timeout_policy_classifies_join_idle_and_drain() {
        let policy = SessionTimeoutPolicy {
            join: Duration::from_secs(5),
            idle_active: Duration::from_secs(10),
            drain: Duration::from_secs(3),
        };

        assert_eq!(
            evaluate_session_timeout(SessionPhase::Joining, Duration::from_secs(5), 0, &policy),
            Some(SessionTimeoutAction::Transition(
                SessionPhaseEvent::JoinTimedOut
            ))
        );
        assert_eq!(
            evaluate_session_timeout(SessionPhase::Joining, Duration::from_secs(4), 0, &policy),
            None
        );
        assert_eq!(
            evaluate_session_timeout(SessionPhase::Active, Duration::from_secs(10), 0, &policy),
            Some(SessionTimeoutAction::Transition(
                SessionPhaseEvent::IdleTimedOut
            ))
        );
        assert_eq!(
            evaluate_session_timeout(SessionPhase::Active, Duration::from_secs(10), 1, &policy),
            None,
            "active with streams is not idle"
        );
        assert_eq!(
            evaluate_session_timeout(SessionPhase::Draining, Duration::from_secs(3), 0, &policy),
            Some(SessionTimeoutAction::Transition(
                SessionPhaseEvent::DrainTimedOut
            ))
        );
    }

    #[test]
    fn zero_budgets_disable_timeouts() {
        let policy = SessionTimeoutPolicy {
            join: Duration::ZERO,
            idle_active: Duration::ZERO,
            drain: Duration::ZERO,
        };
        assert_eq!(
            evaluate_session_timeout(SessionPhase::Joining, Duration::from_secs(999), 0, &policy),
            None
        );
    }

    #[test]
    fn reconnect_allowed_when_absent_or_draining() {
        assert_eq!(evaluate_reconnect(None), ReconnectEligibility::Allowed);
        assert_eq!(
            evaluate_reconnect(Some(SessionPhase::Draining)),
            ReconnectEligibility::Allowed
        );
        assert_eq!(
            evaluate_reconnect(Some(SessionPhase::Active)),
            ReconnectEligibility::Denied
        );
        assert_eq!(
            evaluate_reconnect(Some(SessionPhase::Joining)),
            ReconnectEligibility::Denied
        );
    }
}
