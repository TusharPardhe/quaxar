//! Reference oracle for the pruned node store.
//!
//! `ModelStore` is a deliberately simple, in-memory model of what the real
//! index must achieve: given the sequence of validated ledgers and a retention
//! window, it computes the exact set of node hashes that must remain on disk.
//! It does none of the incremental bookkeeping (no notebook, no counts) the
//! real [`IndexWriter`](super::index) does; it just records each ledger's full
//! node set and derives the required set by union over the retained window.
//!
//! Later stages run random ledger sequences (edits, reversions, gaps, forks)
//! through both the real writer and this oracle and assert the writer's live
//! set never drops a node the oracle still requires. The oracle is the
//! definition of correctness, so it is kept obviously-correct rather than fast.

use basics::base_uint::Uint256;
use std::collections::{BTreeMap, BTreeSet};

/// One validated ledger's node footprint, as the model sees it.
///
/// `state` is the set of SHAMap state-tree node hashes reachable from this
/// ledger's state root (inner nodes and account leaves). `owned` is the set of
/// nodes that belong to this ledger alone and are reused by no later ledger:
/// transaction-tree nodes and the ledger header object.
#[derive(Debug, Clone, Default)]
pub struct LedgerSnapshot {
    pub seq: u32,
    pub state: BTreeSet<Uint256>,
    pub owned: BTreeSet<Uint256>,
}

impl LedgerSnapshot {
    pub fn new(seq: u32) -> Self {
        Self {
            seq,
            state: BTreeSet::new(),
            owned: BTreeSet::new(),
        }
    }

    pub fn with_state<I: IntoIterator<Item = Uint256>>(mut self, nodes: I) -> Self {
        self.state = nodes.into_iter().collect();
        self
    }

    pub fn with_owned<I: IntoIterator<Item = Uint256>>(mut self, nodes: I) -> Self {
        self.owned = nodes.into_iter().collect();
        self
    }
}

/// Ground-truth model of required node residency over a retention window.
#[derive(Debug, Default)]
pub struct ModelStore {
    /// Every claimed ledger, keyed by sequence. The model keeps full history
    /// so it can recompute the required set for any window; the real store
    /// does not, which is the whole point of the index.
    ledgers: BTreeMap<u32, LedgerSnapshot>,
    /// Highest claimed sequence.
    claimed_seq: Option<u32>,
}

impl ModelStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a validated ledger. Claims must be monotonic in sequence, which
    /// mirrors the real writer's publish-order claim contract.
    pub fn claim(&mut self, snapshot: LedgerSnapshot) {
        if let Some(prev) = self.claimed_seq {
            assert!(
                snapshot.seq > prev,
                "ModelStore claims must be strictly increasing: {} after {prev}",
                snapshot.seq
            );
        }
        self.claimed_seq = Some(snapshot.seq);
        self.ledgers.insert(snapshot.seq, snapshot);
    }

    pub fn claimed_seq(&self) -> Option<u32> {
        self.claimed_seq
    }

    /// The oldest sequence that must be retained for a window of `online_delete`
    /// ledgers, capped by `can_delete`. Returns `None` when nothing is claimed.
    ///
    /// The window alone keeps `[claimed - online_delete + 1, claimed]`. The
    /// prune target `K` (oldest ledger dropped) is capped by `can_delete`
    /// (`K = min(claimed - online_delete, can_delete)`), so a smaller
    /// `can_delete` keeps *more* history. `can_delete = u32::MAX` imposes no
    /// cap. The retained floor is therefore
    /// `max(first, min(window_floor, can_delete + 1))`.
    pub fn retained_floor(&self, online_delete: u32, can_delete: u32) -> Option<u32> {
        let claimed = self.claimed_seq?;
        let first = *self.ledgers.keys().next()?;
        let window_floor = claimed.saturating_sub(online_delete.saturating_sub(1));
        // Everything at or below `can_delete` may be pruned, so the lowest the
        // floor may reach under the advisory cap is `can_delete + 1`.
        let advisory_floor = can_delete.saturating_add(1);
        Some(first.max(window_floor.min(advisory_floor)))
    }

    /// The exact set of node hashes that must remain for the given window.
    ///
    /// Union of every retained ledger's state set and owned set. A node shared
    /// by several ledgers appears once. Anything not in this set is safe to
    /// delete.
    pub fn required_nodes(&self, online_delete: u32, can_delete: u32) -> BTreeSet<Uint256> {
        let mut required = BTreeSet::new();
        let Some(floor) = self.retained_floor(online_delete, can_delete) else {
            return required;
        };
        for (seq, snapshot) in self.ledgers.range(floor..) {
            debug_assert_eq!(*seq, snapshot.seq);
            required.extend(snapshot.state.iter().copied());
            required.extend(snapshot.owned.iter().copied());
        }
        required
    }

    /// Sequences still retained for the window, ascending. Mirrors the range a
    /// node should advertise as complete.
    pub fn retained_sequences(&self, online_delete: u32, can_delete: u32) -> Vec<u32> {
        match self.retained_floor(online_delete, can_delete) {
            Some(floor) => self.ledgers.range(floor..).map(|(seq, _)| *seq).collect(),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Uint256 {
        Uint256::from_array([byte; 32])
    }

    /// Build the running example from the design doc: Alice changes every
    /// ledger, Carol (node C) never does. Returns ledgers 1..=n.
    fn alice_carol(n: u32) -> Vec<LedgerSnapshot> {
        let carol = h(0xC0);
        (1..=n)
            .map(|seq| {
                let root = h(seq as u8);
                let alice = h(0xA0u8.wrapping_add(seq as u8));
                LedgerSnapshot::new(seq)
                    .with_state([root, alice, carol])
                    .with_owned([h(0x70u8.wrapping_add(seq as u8))])
            })
            .collect()
    }

    #[test]
    fn empty_store_requires_nothing() {
        let store = ModelStore::new();
        assert!(store.required_nodes(10, u32::MAX).is_empty());
        assert_eq!(store.retained_floor(10, u32::MAX), None);
    }

    #[test]
    fn window_retains_last_n_ledgers_and_shared_carol() {
        let mut store = ModelStore::new();
        for snapshot in alice_carol(6) {
            store.claim(snapshot);
        }
        // Keep 3 ledgers: retained range is 4..=6.
        assert_eq!(store.retained_sequences(3, u32::MAX), vec![4, 5, 6]);
        let required = store.required_nodes(3, u32::MAX);
        // Carol is required (shared by every retained ledger).
        assert!(required.contains(&h(0xC0)));
        // Alice from a pruned ledger (seq 1) is not required.
        assert!(!required.contains(&h(0xA0u8.wrapping_add(1))));
        // Alice from a retained ledger (seq 6) is required.
        assert!(required.contains(&h(0xA0u8.wrapping_add(6))));
        // Owned node of a pruned ledger (seq 1) is gone.
        assert!(!required.contains(&h(0x70u8.wrapping_add(1))));
        // Owned node of a retained ledger (seq 4) remains.
        assert!(required.contains(&h(0x70u8.wrapping_add(4))));
    }

    #[test]
    fn can_delete_caps_the_floor() {
        let mut store = ModelStore::new();
        for snapshot in alice_carol(10) {
            store.claim(snapshot);
        }
        // The window (3) wants to keep 8..=10, i.e. prune 1..=7. can_delete=5
        // forbids pruning above 5, so only 1..=5 may be dropped and the floor
        // settles at 6.
        let floor = store.retained_floor(3, 5).expect("floor");
        assert_eq!(floor, 6);
        assert_eq!(store.retained_sequences(3, 5), vec![6, 7, 8, 9, 10]);
    }

    #[test]
    fn gap_in_sequences_is_handled() {
        let mut store = ModelStore::new();
        store.claim(LedgerSnapshot::new(100).with_state([h(1), h(2)]));
        // Jump to 150 (ledgers 101..149 missing).
        store.claim(LedgerSnapshot::new(150).with_state([h(2), h(3)]));
        // Window of 10 ends at 150; floor is 141, so only ledger 150 is kept.
        assert_eq!(store.retained_sequences(10, u32::MAX), vec![150]);
        let required = store.required_nodes(10, u32::MAX);
        assert!(required.contains(&h(2)));
        assert!(required.contains(&h(3)));
        assert!(!required.contains(&h(1)));
    }

    #[test]
    fn claims_must_increase() {
        let mut store = ModelStore::new();
        store.claim(LedgerSnapshot::new(5));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.claim(LedgerSnapshot::new(5));
        }));
        assert!(result.is_err(), "re-claiming a sequence must panic");
    }
}
