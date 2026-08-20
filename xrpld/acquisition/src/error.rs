//! Error types for the acquisition protocol surface.

use crate::identity::{LiveSessionIdentity, OperationRef, SessionRef};
use crate::phase::{SyncPhase, TransitionFact};

/// An error produced while processing a coordinator event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionError {
    /// A `SyncPhase` transition was illegal from the current phase.
    InvalidPhaseTransition {
        /// The source phase.
        from: SyncPhase,
        /// The motivating fact.
        fact: TransitionFact,
    },

    /// The event referenced a session that is no longer live (cancelled,
    /// replaced, or from another run epoch).
    StaleEvent {
        /// A stable category label for tracing.
        category: &'static str,
    },

    /// The completion's `SessionRef` did not match the live session identity.
    SessionIdentityMismatch {
        /// The session reference carried by the completion.
        received: SessionRef,
        /// The live session identity.
        live: LiveSessionIdentity,
    },

    /// The completion's `OperationRef` was not the exact expected in-flight
    /// operation (wrong id/generation/kind, or a rearmed timer).
    OperationMismatch {
        /// The operation reference carried by the completion.
        received: OperationRef,
        /// The expected operation.
        expected: OperationRef,
    },

    /// The event targeted a session that already reached a terminal phase.
    TerminalSession {
        /// The terminal session.
        session: SessionRef,
    },

    /// The coordinator is shutting down and rejected the event.
    ShuttingDown,
}

impl AcquisitionError {
    /// True when the error is a stale/late completion that must be ignored and
    /// counted as stale rather than treated as a session failure.
    pub const fn is_stale(&self) -> bool {
        matches!(
            self,
            Self::StaleEvent { .. }
                | Self::SessionIdentityMismatch { .. }
                | Self::OperationMismatch { .. }
                | Self::TerminalSession { .. }
        )
    }
}
