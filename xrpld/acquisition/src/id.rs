//! Typed identifiers for the acquisition domain.
//!
//! Every externally dispatched operation carries a complete, comparable set of
//! these IDs (see [`crate::identity`]). A raw `u64` is never sufficient
//! identity for session mutation; these newtypes make mixing an epoch, session,
//! or operation ID at the wrong call site a compile error.

use core::fmt;

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(u64);

        impl $name {
            /// The reserved "no value" identity. Never assigned by [`IdCounter`].
            pub const INVALID: Self = Self(0);

            /// Wraps a raw value. Prefer [`IdCounter`] for fresh identities.
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the wrapped value.
            pub const fn get(self) -> u64 {
                self.0
            }

            /// True when this identity is the reserved invalid value.
            pub const fn is_invalid(self) -> bool {
                self.0 == 0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

id_newtype! {
    /// Identifies one coordinator run. Bumped on restart or a coordinated reset;
    /// any event carrying an older epoch is stale.
    RunEpoch
}

id_newtype! {
    /// Identifies one acquisition session within a run epoch.
    SessionId
}

id_newtype! {
    /// Identifies one tree-plan generation within a session. A retarget creates
    /// a new plan epoch; completions carrying an old plan epoch are stale for
    /// plan-scoped operations.
    PlanEpoch
}

id_newtype! {
    /// Identifies one NodeStore database generation. Store rotation isolates
    /// old and new `(hash, seq, generation)` reads.
    StoreGeneration
}

id_newtype! {
    /// Identifies one dispatched operation of a given kind within a session.
    OperationId
}

id_newtype! {
    /// Guards against a rearmed or late same-kind operation on a still-live
    /// session. A rearm bumps the generation; a completion carrying an older
    /// generation for the same kind is stale.
    OperationGeneration
}

id_newtype! {
    /// Unique identity of one durable ledger handoff. The recipient deduplicates
    /// by this id so transport retries cannot duplicate adoption or publication.
    DurableHandoffId
}

id_newtype! {
    /// Monotonic identity of one packet admission lease.
    AdmissionLeaseId
}

id_newtype! {
    /// Monotonic identity of one routing snapshot publication.
    RoutingGeneration
}

id_newtype! {
    /// Identifies one network peer in overlay-facing events.
    PeerId
}

/// Deterministic source of fresh typed IDs.
///
/// Starts at 1 so the reserved `INVALID` (0) value is never assigned. Not
/// synchronized; the coordinator and its deterministic tests own a single
/// counter per domain, mirroring how session ids are minted on one owner.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdCounter {
    next: u64,
}

impl IdCounter {
    /// A counter whose first `next` yields value 1.
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    /// Returns the next fresh identity of type `T`.
    pub fn next_id<T: From<u64>>(&mut self) -> T {
        let value = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        T::from(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_never_yields_invalid_and_is_monotonic() {
        let mut counter = IdCounter::new();
        let first = counter.next_id::<SessionId>();
        assert!(!first.is_invalid());
        assert_eq!(first, SessionId::new(1));
        let second = counter.next_id::<SessionId>();
        assert!(second > first);
    }

    #[test]
    fn ids_are_distinct_newtypes() {
        // Mixing a session id with a run epoch is a compile error; the wrapped
        // values compare only within their own type.
        let epoch = RunEpoch::new(7);
        let session = SessionId::new(7);
        assert_eq!(epoch.get(), session.get());
        assert_ne!(epoch, RunEpoch::new(8));
        assert_eq!(format!("{epoch}"), "RunEpoch(7)");
    }

    #[test]
    fn invalid_is_zero() {
        assert!(RunEpoch::INVALID.is_invalid());
        assert_eq!(RunEpoch::default(), RunEpoch::INVALID);
    }
}
