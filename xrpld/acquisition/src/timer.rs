//! Timer kinds and timer requests.
//!
//! Timer threads must never run session logic: the coordinator arms a typed
//! [`TimerRequest`] through the timer port, and a wakeup returns as a typed
//! [`crate::AcquisitionEvent::TimerFired`] carrying the exact operation that
//! armed it.

use std::time::Duration;

use crate::identity::OperationRef;

/// The kind of timer a coordinator arms. Each kind has distinct cancellation
/// and rearm semantics on the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerKind {
    /// Acquisition attempt deadline for a session.
    AcquireTimeout,
    /// Backoff before the next read/plan retry.
    ReadRetry,
    /// Retry of a not-yet-acknowledged durable handoff.
    HandoffRetry,
    /// Periodic reassertion of the service phase.
    PhaseReassert,
}

impl TimerKind {
    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AcquireTimeout => "acquire_timeout",
            Self::ReadRetry => "read_retry",
            Self::HandoffRetry => "handoff_retry",
            Self::PhaseReassert => "phase_reassert",
        }
    }
}

/// An arm request produced by the coordinator. `operation` identifies the exact
/// timer; a `TimerFired` completion is matched against it with
/// [`OperationRef::is_expected_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerRequest {
    operation: OperationRef,
    timer: TimerKind,
    after: Duration,
}

impl TimerRequest {
    /// Builds a timer request.
    pub const fn new(operation: OperationRef, timer: TimerKind, after: Duration) -> Self {
        Self {
            operation,
            timer,
            after,
        }
    }

    /// The exact operation this timer belongs to.
    pub const fn operation(self) -> OperationRef {
        self.operation
    }

    /// The timer kind.
    pub const fn timer(self) -> TimerKind {
        self.timer
    }

    /// The delay before the wakeup.
    pub const fn after(self) -> Duration {
        self.after
    }
}
