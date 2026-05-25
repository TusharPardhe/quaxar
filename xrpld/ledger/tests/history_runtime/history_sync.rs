use basics::base_uint::Uint256;
use basics::range_set::{RangeSet, range};
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    FETCH_PACK_STALE_AFTER, HistoryAdvancePlan, HistoryFetchPeer, HistoryHashLookup,
    HistoryInboundAcquire, HistoryLedgerLookup, HistorySqlInfo, InboundLedgerReason, LedgerHeader,
    LedgerHistorySyncConfig, LedgerHistorySyncState, apply_fill_plan,
    expire_stale_fetch_pack_request, run_history_advance, should_acquire,
    should_drop_fetch_pack_request,
};
use overlay::ProtocolPayload;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use time::Duration;

fn sample_hash(value: u32) -> SHAMapHash {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&value.to_be_bytes());
    SHAMapHash::new(Uint256::from_array(bytes))
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

#[derive(Clone, Copy)]
struct Peer {
    min: u32,
    max: u32,
    clustered_score: i32,
    unclustered_score: i32,
}

impl HistoryFetchPeer for Peer {
    fn has_range(&self, min_sequence: u32, max_sequence: u32) -> bool {
        self.min <= min_sequence && self.max >= max_sequence
    }

    fn score(&self, clustered: bool) -> i32 {
        if clustered {
            self.clustered_score
        } else {
            self.unclustered_score
        }
    }
}

#[test]
fn history_sync_builds_fetch_pack_request_for_best_peer() {
    let missing = 101;
    let mut complete_ledgers = RangeSet::new();
    complete_ledgers.insert_interval(range(98, 100));
    complete_ledgers.insert_interval(range(102, 102));
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            (
                (101, reason_key(InboundLedgerReason::History)),
                sample_hash(101),
            ),
            (
                (102, reason_key(InboundLedgerReason::History)),
                sample_hash(102),
            ),
            (
                (100, reason_key(InboundLedgerReason::History)),
                sample_hash(100),
            ),
        ]),
    };
    let peers = vec![
        Peer {
            min: 50,
            max: 103,
            clustered_score: 10,
            unclustered_score: 30,
        },
        Peer {
            min: 100,
            max: 103,
            clustered_score: 25,
            unclustered_score: 5,
        },
    ];

    let plan: HistoryAdvancePlan<LedgerHeader> = run_history_advance(
        102,
        102,
        None,
        InboundLedgerReason::History,
        Duration::seconds(10),
        LedgerHistorySyncConfig::new(256, 3, FETCH_PACK_STALE_AFTER),
        &LedgerHistorySyncState {
            complete_ledgers,
            fetch_state: Default::default(),
            fetch_pack_issued_at: None,
        },
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 100,
            hashes: BTreeMap::new(),
        },
        &peers,
        true,
    );

    assert_eq!(plan.missing, Some(missing));
    assert_eq!(
        plan.next_state.fetch_pack_issued_at,
        Some(Duration::seconds(10))
    );
    let fetch_pack = plan.fetch_pack.expect("fetch pack request");
    assert_eq!(fetch_pack.peer_index, 1);
    assert_eq!(fetch_pack.missing, missing);
    assert_eq!(fetch_pack.ledger_hash, sample_hash(102));
    match fetch_pack.message.payload {
        ProtocolPayload::GetObjects(request) => {
            assert_eq!(request.r#type, 6);
            assert!(request.query);
            assert_eq!(
                request.ledger_hash,
                Some(sample_hash(102).as_uint256().data().to_vec())
            );
            assert!(request.objects.is_empty());
        }
        payload => panic!("expected get_objects payload, got {payload:?}"),
    }
    assert_eq!(
        plan.prefetch,
        vec![
            (101, sample_hash(101), InboundLedgerReason::History),
            (100, sample_hash(100), InboundLedgerReason::History),
        ]
    );
}

#[test]
fn history_sync_uses_cluster_mode_to_choose_different_fetch_pack_peer() {
    let missing = 101;
    let mut complete_ledgers = RangeSet::new();
    complete_ledgers.insert_interval(range(98, 100));
    complete_ledgers.insert_interval(range(102, 102));
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            (
                (101, reason_key(InboundLedgerReason::History)),
                sample_hash(101),
            ),
            (
                (102, reason_key(InboundLedgerReason::History)),
                sample_hash(102),
            ),
            (
                (100, reason_key(InboundLedgerReason::History)),
                sample_hash(100),
            ),
        ]),
    };
    let peers = vec![
        Peer {
            min: 50,
            max: 102,
            clustered_score: 10,
            unclustered_score: 30,
        },
        Peer {
            min: 100,
            max: 103,
            clustered_score: 25,
            unclustered_score: 5,
        },
    ];

    let plan: HistoryAdvancePlan<LedgerHeader> = run_history_advance(
        102,
        102,
        None,
        InboundLedgerReason::History,
        Duration::seconds(10),
        LedgerHistorySyncConfig::new(256, 3, FETCH_PACK_STALE_AFTER),
        &LedgerHistorySyncState {
            complete_ledgers,
            fetch_state: Default::default(),
            fetch_pack_issued_at: None,
        },
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 100,
            hashes: BTreeMap::new(),
        },
        &peers,
        false,
    );

    let fetch_pack = plan.fetch_pack.expect("fetch pack request");
    assert_eq!(fetch_pack.peer_index, 0);
    assert_eq!(fetch_pack.missing, missing);
}

#[test]
fn history_sync_stale_fetch_pack_requests_drop_after_one_second() {
    assert!(!should_drop_fetch_pack_request(
        Duration::seconds(5),
        Duration::seconds(5) + FETCH_PACK_STALE_AFTER
    ));
    assert!(should_drop_fetch_pack_request(
        Duration::seconds(5),
        Duration::seconds(7)
    ));
}

#[test]
fn history_sync_apply_fill_plan_marks_ranges_complete_and_clears_fill_flag() {
    let mut state = LedgerHistorySyncState::<LedgerHeader> {
        complete_ledgers: RangeSet::new(),
        fetch_state: ledger::FetchForHistoryState {
            fill_in_progress: 250,
            ..Default::default()
        },
        fetch_pack_issued_at: None,
    };

    apply_fill_plan(
        &mut state,
        250,
        &ledger::LedgerHistoryFillPlan {
            inserted_ranges: vec![
                ledger::LedgerFillRange { min: 200, max: 210 },
                ledger::LedgerFillRange { min: 198, max: 199 },
            ],
            stop_reason: ledger::LedgerHistoryFillStopReason::ReachedGenesis,
        },
    );

    assert!(state.complete_ledgers.contains(198));
    assert!(state.complete_ledgers.contains(210));
    assert_eq!(state.fetch_state.fill_in_progress, 0);
}

#[test]
fn history_sync_expires_stale_fetch_pack_state_and_allows_retry() {
    let mut state = LedgerHistorySyncState::<LedgerHeader> {
        fetch_state: ledger::FetchForHistoryState {
            fetch_seq: 41,
            ..Default::default()
        },
        fetch_pack_issued_at: Some(Duration::seconds(5)),
        ..Default::default()
    };

    assert!(expire_stale_fetch_pack_request(
        &mut state,
        Duration::seconds(7),
        FETCH_PACK_STALE_AFTER
    ));
    assert_eq!(state.fetch_state.fetch_seq, 0);
    assert_eq!(state.fetch_pack_issued_at, None);
}

#[test]
fn history_sync_reissues_fetch_pack_after_stale_expiry() {
    let missing = 101;
    let mut complete_ledgers = RangeSet::new();
    complete_ledgers.insert_interval(range(98, 100));
    complete_ledgers.insert_interval(range(102, 102));
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            (
                (101, reason_key(InboundLedgerReason::History)),
                sample_hash(101),
            ),
            (
                (102, reason_key(InboundLedgerReason::History)),
                sample_hash(102),
            ),
            (
                (100, reason_key(InboundLedgerReason::History)),
                sample_hash(100),
            ),
        ]),
    };
    let peers = vec![Peer {
        min: 100,
        max: 103,
        clustered_score: 25,
        unclustered_score: 25,
    }];

    let plan: HistoryAdvancePlan<LedgerHeader> = run_history_advance(
        102,
        102,
        None,
        InboundLedgerReason::History,
        Duration::seconds(7),
        LedgerHistorySyncConfig::new(256, 3, FETCH_PACK_STALE_AFTER),
        &LedgerHistorySyncState {
            complete_ledgers,
            fetch_state: ledger::FetchForHistoryState {
                fetch_seq: missing,
                ..Default::default()
            },
            fetch_pack_issued_at: Some(Duration::seconds(5)),
        },
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 100,
            hashes: BTreeMap::new(),
        },
        &peers,
        true,
    );

    assert_eq!(plan.next_state.fetch_state.fetch_seq, missing);
    assert_eq!(
        plan.next_state.fetch_pack_issued_at,
        Some(Duration::seconds(7))
    );
    assert!(plan.fetch_pack.is_some());
}

#[test]
fn should_acquire_policy_order() {
    assert!(should_acquire(500, 256, None, 500));
    assert!(should_acquire(500, 256, None, 300));
    assert!(!should_acquire(500, 50, None, 300));
    assert!(should_acquire(500, 50, Some(300), 300));
}

#[test]
fn history_sync_skips_zero_hash_fetch_pack() {
    let missing = 101;
    let mut complete_ledgers = RangeSet::new();
    complete_ledgers.insert_interval(range(98, 100));
    complete_ledgers.insert_interval(range(102, 102));
    let hashes = RecordingHashes {
        values: BTreeMap::from([
            (
                (101, reason_key(InboundLedgerReason::History)),
                sample_hash(101),
            ),
            (
                (102, reason_key(InboundLedgerReason::History)),
                sample_hash(0),
            ),
        ]),
    };
    let peers = vec![Peer {
        min: 100,
        max: 103,
        clustered_score: 25,
        unclustered_score: 25,
    }];

    let plan: HistoryAdvancePlan<LedgerHeader> = run_history_advance(
        102,
        102,
        None,
        InboundLedgerReason::History,
        Duration::seconds(10),
        LedgerHistorySyncConfig::new(256, 3, FETCH_PACK_STALE_AFTER),
        &LedgerHistorySyncState {
            complete_ledgers,
            fetch_state: Default::default(),
            fetch_pack_issued_at: None,
        },
        &hashes,
        &RecordingLookup::default(),
        &RecordingInbound::default(),
        &RecordingSql {
            earliest: 100,
            hashes: BTreeMap::new(),
        },
        &peers,
        true,
    );

    assert_eq!(plan.missing, Some(missing));
    assert!(plan.fetch_pack.is_none());
    assert_eq!(plan.next_state.fetch_pack_issued_at, None);
}
