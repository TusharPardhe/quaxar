use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    LedgerFillRange, LedgerHashPair, LedgerHashPairProvider, LedgerHeader,
    LedgerHistoryFillStopReason, LedgerObjectPresence, LedgerPresence, Stopper,
    run_try_fill_backwalk,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

fn sample_hash(value: u32) -> SHAMapHash {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&value.to_be_bytes());
    SHAMapHash::new(Uint256::from_array(bytes))
}

#[derive(Default)]
struct RecordingPresence {
    have: HashSet<u32>,
}

impl LedgerPresence for RecordingPresence {
    fn have_ledger(&self, ledger_index: u32) -> bool {
        self.have.contains(&ledger_index)
    }
}

#[derive(Default)]
struct RecordingHashPairs {
    entries: BTreeMap<u32, LedgerHashPair>,
    calls: RefCell<Vec<(u32, u32)>>,
}

impl LedgerHashPairProvider for RecordingHashPairs {
    fn get_hashes_by_index(&self, min_seq: u32, max_seq: u32) -> Vec<(u32, LedgerHashPair)> {
        self.calls.borrow_mut().push((min_seq, max_seq));
        self.entries
            .range(min_seq..=max_seq)
            .map(|(seq, pair)| (*seq, *pair))
            .collect()
    }
}

#[derive(Default)]
struct RecordingNodeStore {
    present: BTreeMap<u32, SHAMapHash>,
    checks: RefCell<Vec<(u32, SHAMapHash)>>,
}

impl LedgerObjectPresence for RecordingNodeStore {
    fn has_ledger_object(&self, ledger_hash: SHAMapHash, ledger_seq: u32) -> bool {
        self.checks.borrow_mut().push((ledger_seq, ledger_hash));
        self.present.get(&ledger_seq) == Some(&ledger_hash)
    }
}

#[derive(Default)]
struct StaticStopper {
    stopping: bool,
}

impl Stopper for StaticStopper {
    fn is_stopping(&self) -> bool {
        self.stopping
    }
}

fn pair_for(seq: u32) -> LedgerHashPair {
    LedgerHashPair {
        ledger_hash: sample_hash(seq),
        parent_hash: sample_hash(seq.saturating_sub(1)),
    }
}

fn acquired_header(seq: u32) -> LedgerHeader {
    LedgerHeader {
        seq,
        parent_hash: sample_hash(seq - 1),
        ..LedgerHeader::default()
    }
}

#[test]
fn try_fill_stops_when_previous_ledger_is_already_present() {
    let presence = RecordingPresence {
        have: HashSet::from([599]),
    };
    let provider = RecordingHashPairs::default();
    let node_store = RecordingNodeStore::default();

    let plan = run_try_fill_backwalk(
        &acquired_header(600),
        &presence,
        &provider,
        &node_store,
        &StaticStopper::default(),
    );
    assert_eq!(
        plan.inserted_ranges,
        vec![LedgerFillRange { min: 600, max: 600 }]
    );
    assert_eq!(
        plan.stop_reason,
        LedgerHistoryFillStopReason::AlreadyHaveLedger { seq: 599 }
    );
    assert!(provider.calls.borrow().is_empty());
    assert!(node_store.checks.borrow().is_empty());
}

#[test]
fn try_fill_refreshes_sql_window_in_500_ledger_chunks() {
    let presence = RecordingPresence {
        have: HashSet::from([97]),
    };
    let mut entries = BTreeMap::new();
    for seq in 98..=599 {
        entries.insert(seq, pair_for(seq));
    }
    let provider = RecordingHashPairs {
        entries,
        calls: RefCell::default(),
    };
    let node_store = RecordingNodeStore {
        present: BTreeMap::from([(100, sample_hash(100)), (98, sample_hash(98))]),
        checks: RefCell::default(),
    };

    let plan = run_try_fill_backwalk(
        &acquired_header(600),
        &presence,
        &provider,
        &node_store,
        &StaticStopper::default(),
    );
    assert_eq!(*provider.calls.borrow(), vec![(100, 599), (0, 99)]);
    assert_eq!(
        *node_store.checks.borrow(),
        vec![(100, sample_hash(100)), (98, sample_hash(98))]
    );
    assert_eq!(
        plan.inserted_ranges,
        vec![
            LedgerFillRange { min: 600, max: 600 },
            LedgerFillRange { min: 100, max: 600 },
            LedgerFillRange { min: 98, max: 100 },
        ]
    );
    assert_eq!(
        plan.stop_reason,
        LedgerHistoryFillStopReason::AlreadyHaveLedger { seq: 97 }
    );
}

#[test]
fn try_fill_stops_on_missing_sql_row_and_keeps_last_complete_range() {
    let presence = RecordingPresence {
        have: HashSet::from([97]),
    };
    let mut entries = BTreeMap::new();
    for seq in 99..=599 {
        entries.insert(seq, pair_for(seq));
    }
    let provider = RecordingHashPairs {
        entries,
        calls: RefCell::default(),
    };
    let node_store = RecordingNodeStore {
        present: BTreeMap::from([(100, sample_hash(100)), (99, sample_hash(99))]),
        checks: RefCell::default(),
    };

    let plan = run_try_fill_backwalk(
        &acquired_header(600),
        &presence,
        &provider,
        &node_store,
        &StaticStopper::default(),
    );

    assert_eq!(*provider.calls.borrow(), vec![(100, 599), (0, 99), (0, 98)]);
    assert_eq!(
        plan.inserted_ranges,
        vec![
            LedgerFillRange { min: 600, max: 600 },
            LedgerFillRange { min: 100, max: 600 },
            LedgerFillRange { min: 99, max: 100 },
            LedgerFillRange { min: 99, max: 99 },
        ]
    );
    assert_eq!(
        plan.stop_reason,
        LedgerHistoryFillStopReason::MissingSqlWindow { seq: 98 }
    );
}

#[test]
fn try_fill_stops_on_node_store_mismatch() {
    let presence = RecordingPresence::default();
    let mut entries = BTreeMap::new();
    for seq in 100..=599 {
        entries.insert(seq, pair_for(seq));
    }
    let provider = RecordingHashPairs {
        entries,
        calls: RefCell::default(),
    };
    let node_store = RecordingNodeStore::default();

    let plan = run_try_fill_backwalk(
        &acquired_header(600),
        &presence,
        &provider,
        &node_store,
        &StaticStopper::default(),
    );

    assert_eq!(*node_store.checks.borrow(), vec![(100, sample_hash(100))]);
    assert_eq!(
        plan.inserted_ranges,
        vec![
            LedgerFillRange { min: 600, max: 600 },
            LedgerFillRange { min: 600, max: 600 },
        ]
    );
    assert_eq!(
        plan.stop_reason,
        LedgerHistoryFillStopReason::NodeStoreMismatch { seq: 599 }
    );
}

#[test]
fn try_fill_stops_on_parent_hash_mismatch() {
    let presence = RecordingPresence::default();
    let mut entries = BTreeMap::new();
    entries.insert(
        19,
        LedgerHashPair {
            ledger_hash: sample_hash(0xdead_beef),
            parent_hash: sample_hash(18),
        },
    );
    let provider = RecordingHashPairs {
        entries,
        calls: RefCell::default(),
    };
    let node_store = RecordingNodeStore {
        present: BTreeMap::from([(19, sample_hash(0xdead_beef))]),
        checks: RefCell::default(),
    };

    let plan = run_try_fill_backwalk(
        &acquired_header(20),
        &presence,
        &provider,
        &node_store,
        &StaticStopper::default(),
    );

    assert_eq!(
        plan.inserted_ranges,
        vec![
            LedgerFillRange { min: 20, max: 20 },
            LedgerFillRange { min: 20, max: 20 },
        ]
    );
    assert_eq!(
        plan.stop_reason,
        LedgerHistoryFillStopReason::ParentHashMismatch {
            seq: 19,
            expected: sample_hash(19),
            found: sample_hash(0xdead_beef),
        }
    );
}
