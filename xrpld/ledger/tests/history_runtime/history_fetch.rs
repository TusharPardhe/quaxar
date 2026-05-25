use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    FetchForHistoryState, FetchPackRequest, HistoryHashLookup, HistoryInboundAcquire,
    HistoryLedgerLookup, HistorySqlInfo, InboundLedgerReason, LedgerHeader, PrefetchAcquire,
    run_fetch_for_history,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

fn sample_hash(value: u32) -> SHAMapHash {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&value.to_be_bytes());
    SHAMapHash::new(Uint256::from_array(bytes))
}

fn header(seq: u32) -> LedgerHeader {
    LedgerHeader {
        seq,
        hash: sample_hash(seq),
        parent_hash: sample_hash(seq.saturating_sub(1)),
        ..LedgerHeader::default()
    }
}

fn reason_key(reason: InboundLedgerReason) -> u8 {
    match reason {
        InboundLedgerReason::History => 0,
        InboundLedgerReason::Generic => 1,
        InboundLedgerReason::Consensus => 2,
    }
}

#[derive(Default)]
struct RecordingHashes {
    values: BTreeMap<(u32, u8), SHAMapHash>,
}

impl HistoryHashLookup for RecordingHashes {
    fn get_ledger_hash_for_history(
        &self,
        ledger_index: u32,
        reason: InboundLedgerReason,
    ) -> Option<SHAMapHash> {
        self.values
            .get(&(ledger_index, reason_key(reason)))
            .copied()
    }
}

#[derive(Default)]
struct RecordingLookup {
    ledgers: HashMap<SHAMapHash, LedgerHeader>,
}

impl HistoryLedgerLookup<LedgerHeader> for RecordingLookup {
    fn get_ledger_by_hash(&self, hash: SHAMapHash) -> Option<LedgerHeader> {
        self.ledgers.get(&hash).copied()
    }
}

#[derive(Default)]
struct RecordingInbound {
    failures: HashSet<SHAMapHash>,
    acquired: BTreeMap<(SHAMapHash, u32, u8), LedgerHeader>,
    calls: RefCell<Vec<(SHAMapHash, u32, InboundLedgerReason)>>,
}

impl HistoryInboundAcquire<LedgerHeader> for RecordingInbound {
    fn is_failure(&self, hash: SHAMapHash) -> bool {
        self.failures.contains(&hash)
    }

    fn acquire(
        &self,
        hash: SHAMapHash,
        seq: u32,
        reason: InboundLedgerReason,
    ) -> Option<LedgerHeader> {
        self.calls.borrow_mut().push((hash, seq, reason));
        self.acquired.get(&(hash, seq, reason_key(reason))).copied()
    }
}

#[derive(Default)]
struct RecordingSql {
    earliest: u32,
    hashes: BTreeMap<u32, SHAMapHash>,
}

impl HistorySqlInfo for RecordingSql {
    fn earliest_ledger_seq(&self) -> u32 {
        self.earliest
    }

    fn get_hash_by_index(&self, ledger_index: u32) -> SHAMapHash {
        self.hashes.get(&ledger_index).copied().unwrap_or_default()
    }
}

#[test]
fn fetch_for_history_uses_cached_ledger_before_inbound_acquire() {
    let cached = header(20);
    let hashes = RecordingHashes {
        values: BTreeMap::from([((20, reason_key(InboundLedgerReason::History)), cached.hash)]),
    };
    let lookup = RecordingLookup {
        ledgers: HashMap::from([(cached.hash, cached)]),
    };

    let result = run_fetch_for_history(
        20,
        InboundLedgerReason::History,
        4,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &lookup,
        &RecordingInbound::default(),
        &RecordingSql::default(),
    );

    assert!(result.progress);
    assert_eq!(result.set_full_ledger, Some(cached));
    assert_eq!(result.next_state.hist_ledger, Some(cached));
    assert!(result.schedule_try_fill.is_none());
}

#[test]
fn fetch_for_history_schedules_try_fill_when_previous_sql_hash_matches() {
    let acquired = header(30);
    let hashes = RecordingHashes {
        values: BTreeMap::from([(
            (30, reason_key(InboundLedgerReason::History)),
            acquired.hash,
        )]),
    };
    let inbound = RecordingInbound {
        failures: HashSet::new(),
        acquired: BTreeMap::from([(
            (acquired.hash, 30, reason_key(InboundLedgerReason::History)),
            acquired,
        )]),
        calls: RefCell::default(),
    };
    let sql = RecordingSql {
        earliest: 1,
        hashes: BTreeMap::from([(29, acquired.parent_hash)]),
    };

    let result = run_fetch_for_history(
        30,
        InboundLedgerReason::History,
        4,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &RecordingLookup::default(),
        &inbound,
        &sql,
    );

    assert!(result.progress);
    assert_eq!(result.set_full_ledger, Some(acquired));
    assert_eq!(result.schedule_try_fill, Some(acquired));
    assert_eq!(result.next_state.fill_in_progress, 30);
    assert_eq!(result.next_state.hist_ledger, Some(acquired));
}

#[test]
fn fetch_for_history_requests_fetch_pack_and_prefetches_when_acquire_fails() {
    let missing = 40;
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            (
                (40, reason_key(InboundLedgerReason::History)),
                sample_hash(40),
            ),
            (
                (39, reason_key(InboundLedgerReason::History)),
                sample_hash(39),
            ),
            (
                (38, reason_key(InboundLedgerReason::History)),
                sample_hash(38),
            ),
        ]),
    };
    let inbound = RecordingInbound::default();
    let sql = RecordingSql {
        earliest: 38,
        hashes: BTreeMap::new(),
    };

    let result = run_fetch_for_history(
        missing,
        InboundLedgerReason::History,
        3,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &RecordingLookup::default(),
        &inbound,
        &sql,
    );

    assert!(!result.progress);
    assert_eq!(
        result.request_fetch_pack,
        Some(FetchPackRequest {
            missing,
            reason: InboundLedgerReason::History,
        })
    );
    assert_eq!(result.next_state.fetch_seq, missing);
    assert_eq!(
        result.prefetch,
        vec![
            PrefetchAcquire {
                seq: 40,
                hash: sample_hash(40),
                reason: InboundLedgerReason::History,
            },
            PrefetchAcquire {
                seq: 39,
                hash: sample_hash(39),
                reason: InboundLedgerReason::History,
            },
            PrefetchAcquire {
                seq: 38,
                hash: sample_hash(38),
                reason: InboundLedgerReason::History,
            },
        ]
    );
}

#[test]
fn fetch_for_history_skips_fetch_pack_after_failure_but_still_prefetches_lower_ledgers() {
    let missing = 25;
    let hash = sample_hash(missing);
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            ((25, reason_key(InboundLedgerReason::History)), hash),
            (
                (24, reason_key(InboundLedgerReason::History)),
                sample_hash(24),
            ),
        ]),
    };
    let inbound = RecordingInbound {
        failures: HashSet::from([hash]),
        acquired: BTreeMap::new(),
        calls: RefCell::default(),
    };
    let sql = RecordingSql {
        earliest: 24,
        hashes: BTreeMap::new(),
    };

    let result = run_fetch_for_history(
        missing,
        InboundLedgerReason::History,
        2,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &RecordingLookup::default(),
        &inbound,
        &sql,
    );

    assert!(result.request_fetch_pack.is_none());
    assert_eq!(
        result.prefetch,
        vec![
            PrefetchAcquire {
                seq: 25,
                hash,
                reason: InboundLedgerReason::History,
            },
            PrefetchAcquire {
                seq: 24,
                hash: sample_hash(24),
                reason: InboundLedgerReason::History,
            },
        ]
    );
    assert!(inbound.calls.borrow().is_empty());
}

#[test]
fn fetch_for_history_suppresses_duplicate_fetch_pack_requests() {
    let missing = 50;
    let hashes = RecordingHashes {
        values: BTreeMap::from([(
            (50, reason_key(InboundLedgerReason::History)),
            sample_hash(50),
        )]),
    };

    let result = run_fetch_for_history(
        missing,
        InboundLedgerReason::History,
        2,
        &FetchForHistoryState {
            fetch_seq: missing,
            ..FetchForHistoryState::<LedgerHeader>::default()
        },
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 1,
            hashes: BTreeMap::new(),
        },
    );

    assert!(result.request_fetch_pack.is_none());
    assert_eq!(result.next_state.fetch_seq, missing);
}

#[test]
fn fetch_for_history_suppresses_fetch_pack_at_or_before_earliest_seq() {
    let missing = 5;
    let hashes = RecordingHashes {
        values: BTreeMap::from([(
            (5, reason_key(InboundLedgerReason::History)),
            sample_hash(5),
        )]),
    };

    let result = run_fetch_for_history(
        missing,
        InboundLedgerReason::History,
        4,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 5,
            hashes: BTreeMap::new(),
        },
    );

    assert!(result.request_fetch_pack.is_none());
    assert_eq!(
        result.prefetch,
        vec![PrefetchAcquire {
            seq: 5,
            hash: sample_hash(5),
            reason: InboundLedgerReason::History,
        }]
    );
}

#[test]
fn fetch_for_history_does_not_schedule_try_fill_when_fill_already_running() {
    let acquired = header(60);
    let hashes = RecordingHashes {
        values: BTreeMap::from([(
            (60, reason_key(InboundLedgerReason::History)),
            acquired.hash,
        )]),
    };
    let inbound = RecordingInbound {
        failures: HashSet::new(),
        acquired: BTreeMap::from([(
            (acquired.hash, 60, reason_key(InboundLedgerReason::History)),
            acquired,
        )]),
        calls: RefCell::default(),
    };

    let result = run_fetch_for_history(
        60,
        InboundLedgerReason::History,
        4,
        &FetchForHistoryState {
            fill_in_progress: 55,
            ..FetchForHistoryState::<LedgerHeader>::default()
        },
        &hashes,
        &RecordingLookup::default(),
        &inbound,
        &RecordingSql {
            earliest: 1,
            hashes: BTreeMap::from([(59, acquired.parent_hash)]),
        },
    );

    assert!(result.progress);
    assert!(result.schedule_try_fill.is_none());
    assert_eq!(result.next_state.fill_in_progress, 55);
}

#[test]
fn fetch_for_history_clears_next_ledger_when_hash_lookup_fails() {
    let result = run_fetch_for_history(
        90,
        InboundLedgerReason::Generic,
        4,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &RecordingHashes::default(),
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql::default(),
    );

    assert!(result.progress);
    assert!(result.missing_hash);
    assert_eq!(result.clear_ledger, Some(91));
}

#[test]
#[should_panic(expected = "xrpl::LedgerMaster::fetchForHistory : found ledger")]
fn fetch_for_history_rejects_zero_hash() {
    let zero = sample_hash(0);
    let hashes = RecordingHashes {
        values: BTreeMap::from([((91, reason_key(InboundLedgerReason::History)), zero)]),
    };

    let _ = run_fetch_for_history(
        91,
        InboundLedgerReason::History,
        4,
        &FetchForHistoryState::<LedgerHeader>::default(),
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql::default(),
    );
}
