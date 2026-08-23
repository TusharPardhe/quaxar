//! Ledger targets, identities, and acquisition reasons.
//!
//! `AcquireReason` mirrors the application-level `app::...registry::AcquireReason`
//! (Consensus/Generic/History) but is owned by this crate so the acquisition
//! domain never depends on `xrpld/app`. Adapters translate between the two.

use basics::base_uint::Uint256;

/// A ledger the coordinator is asked to acquire. The sequence is optional:
/// consensus targets usually carry a sequence, peer-discovery and validation
/// targets may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LedgerTarget {
    hash: Uint256,
    sequence: Option<u32>,
}

impl LedgerTarget {
    /// Builds a target.
    pub const fn new(hash: Uint256, sequence: Option<u32>) -> Self {
        Self { hash, sequence }
    }

    /// The target ledger hash.
    pub const fn hash(self) -> Uint256 {
        self.hash
    }

    /// The optional target ledger sequence.
    pub const fn sequence(self) -> Option<u32> {
        self.sequence
    }
}

/// A ledger identity reported as installed or published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LedgerIdentity {
    hash: Uint256,
    sequence: u32,
}

impl LedgerIdentity {
    /// Builds a ledger identity.
    pub const fn new(hash: Uint256, sequence: u32) -> Self {
        Self { hash, sequence }
    }

    /// The ledger hash.
    pub const fn hash(self) -> Uint256 {
        self.hash
    }

    /// The ledger sequence.
    pub const fn sequence(self) -> u32 {
        self.sequence
    }
}

/// Why a ledger is being acquired. Mirrors the application `AcquireReason`
/// vocabulary; the coordinator uses the reason to weight admission, prioritize
/// work, and attribute metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AcquireReason {
    /// Consensus / validation path.
    Consensus,
    /// LedgerMaster, catchup, publication.
    Generic,
    /// History fill, sequential catchup.
    History,
}

impl AcquireReason {
    /// All reasons, in priority order (most important first).
    pub const ALL: [AcquireReason; 3] = [Self::Consensus, Self::Generic, Self::History];

    /// A stable label for tracing and metrics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Consensus => "consensus",
            Self::Generic => "generic",
            Self::History => "history",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_and_identity_round_trip() {
        let target = LedgerTarget::new(Uint256::from(1), Some(5));
        assert_eq!(target.hash(), Uint256::from(1));
        assert_eq!(target.sequence(), Some(5));

        let identity = LedgerIdentity::new(Uint256::from(2), 6);
        assert_eq!(identity.hash(), Uint256::from(2));
        assert_eq!(identity.sequence(), 6);
    }

    #[test]
    fn reason_labels_are_stable() {
        assert_eq!(AcquireReason::Consensus.label(), "consensus");
        assert_eq!(AcquireReason::Generic.label(), "generic");
        assert_eq!(AcquireReason::History.label(), "history");
        assert_eq!(
            AcquireReason::ALL,
            [
                AcquireReason::Consensus,
                AcquireReason::Generic,
                AcquireReason::History
            ]
        );
    }
}
