//! Owner-level history acquisition and fetch-pack orchestration above the
//! landed deterministic helper cores.
//!
//! This ports the finishable current the reference implementation history slice:
//! - `shouldAcquire(...)`,
//! - missing-ledger discovery through `prevMissing(...)`,
//! - `fetchForHistory(...)` owner state application,
//! - fetch-pack peer selection and `TMGetObjectByHash` request encoding,
//! - and `tryFill(...)` range application back into owner state.

use crate::{
    FetchForHistoryState, HistoryHashLookup, HistoryInboundAcquire, HistoryLedgerLike,
    HistoryLedgerLookup, HistorySqlInfo, InboundLedgerReason, LedgerFillRange,
    LedgerHistoryFillPlan, LedgerHistorySyncConfig, run_fetch_for_history,
};
use basics::range_set::{RangeSet, prev_missing, range};
use basics::sha_map_hash::SHAMapHash;
use overlay::{ProtocolMessage, ProtocolPayload, TmGetObjectByHash};
use time::Duration;

pub const FETCH_PACK_STALE_AFTER: Duration = Duration::seconds(1);
const TM_GET_OBJECT_BY_HASH_FETCH_PACK: i32 = 6;

pub trait HistoryFetchPeer {
    fn has_range(&self, min_sequence: u32, max_sequence: u32) -> bool;
    fn score(&self, clustered: bool) -> i32;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerHistorySyncState<L> {
    pub complete_ledgers: RangeSet<u32>,
    pub fetch_state: FetchForHistoryState<L>,
    pub fetch_pack_issued_at: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchPackSendPlan {
    pub peer_index: usize,
    pub missing: u32,
    pub ledger_hash: SHAMapHash,
    pub message: ProtocolMessage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryAdvancePlan<L> {
    pub missing: Option<u32>,
    pub next_state: LedgerHistorySyncState<L>,
    pub progress: bool,
    pub clear_ledger: Option<u32>,
    pub set_full_ledger: Option<L>,
    pub schedule_try_fill: Option<L>,
    pub prefetch: Vec<(u32, SHAMapHash, InboundLedgerReason)>,
    pub fetch_pack: Option<FetchPackSendPlan>,
    pub missing_hash: bool,
}

pub fn should_acquire(
    current_ledger: u32,
    ledger_history: u32,
    minimum_online: Option<u32>,
    candidate_ledger: u32,
) -> bool {
    if candidate_ledger >= current_ledger {
        return true;
    }

    if current_ledger - candidate_ledger <= ledger_history {
        return true;
    }

    minimum_online.is_some_and(|minimum| candidate_ledger >= minimum)
}

pub fn should_drop_fetch_pack_request(issued_at: Duration, now: Duration) -> bool {
    now > issued_at + FETCH_PACK_STALE_AFTER
}

pub fn expire_stale_fetch_pack_request<L>(
    state: &mut LedgerHistorySyncState<L>,
    now: Duration,
    stale_after: Duration,
) -> bool {
    let should_drop = state
        .fetch_pack_issued_at
        .is_some_and(|issued_at| now > issued_at + stale_after);
    if should_drop {
        if state.fetch_state.fetch_seq != 0 {
            state.fetch_state.fetch_seq = 0;
        }
        state.fetch_pack_issued_at = None;
    }
    should_drop
}

pub fn make_fetch_pack_request(ledger_hash: SHAMapHash) -> ProtocolMessage {
    ProtocolMessage::new(ProtocolPayload::GetObjects(TmGetObjectByHash {
        r#type: TM_GET_OBJECT_BY_HASH_FETCH_PACK,
        query: true,
        ledger_hash: Some(ledger_hash.as_uint256().data().to_vec()),
        fat: None,
        objects: Vec::new(),
    }))
}

pub fn select_fetch_pack_peer<P>(peers: &[P], missing: u32, clustered: bool) -> Option<usize>
where
    P: HistoryFetchPeer,
{
    let mut best_index = None;
    let mut best_score = i32::MIN;

    for (index, peer) in peers.iter().enumerate() {
        if !peer.has_range(missing, missing + 1) {
            continue;
        }

        let score = peer.score(clustered);
        if best_index.is_none() || score > best_score {
            best_index = Some(index);
            best_score = score;
        }
    }

    best_index
}

pub fn apply_fill_plan<L>(
    state: &mut LedgerHistorySyncState<L>,
    fill_in_progress: u32,
    plan: &LedgerHistoryFillPlan,
) {
    for LedgerFillRange { min, max } in &plan.inserted_ranges {
        state.complete_ledgers.insert_interval(range(*min, *max));
    }

    if fill_in_progress != 0 {
        state.fetch_state.fill_in_progress = 0;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_history_advance<L, H, GL, IA, DB, P>(
    published_ledger_seq: u32,
    validated_ledger_seq: u32,
    minimum_online: Option<u32>,
    reason: InboundLedgerReason,
    now: Duration,
    config: LedgerHistorySyncConfig,
    state: &LedgerHistorySyncState<L>,
    hashes: &H,
    ledger_lookup: &GL,
    inbound: &IA,
    sql: &DB,
    peers: &[P],
    clustered: bool,
) -> HistoryAdvancePlan<L>
where
    L: HistoryLedgerLike,
    H: HistoryHashLookup,
    GL: HistoryLedgerLookup<L>,
    IA: HistoryInboundAcquire<L>,
    DB: HistorySqlInfo,
    P: HistoryFetchPeer,
{
    let mut next_state = state.clone();
    let _ = expire_stale_fetch_pack_request(&mut next_state, now, config.fetch_pack_stale_after);

    let Some(missing) = prev_missing(
        &next_state.complete_ledgers,
        published_ledger_seq,
        sql.earliest_ledger_seq(),
    ) else {
        return HistoryAdvancePlan {
            missing: None,
            next_state,
            progress: false,
            clear_ledger: None,
            set_full_ledger: None,
            schedule_try_fill: None,
            prefetch: Vec::new(),
            fetch_pack: None,
            missing_hash: false,
        };
    };

    if next_state.fetch_state.fill_in_progress != 0
        && missing <= next_state.fetch_state.fill_in_progress
    {
        return HistoryAdvancePlan {
            missing: Some(missing),
            next_state,
            progress: false,
            clear_ledger: None,
            set_full_ledger: None,
            schedule_try_fill: None,
            prefetch: Vec::new(),
            fetch_pack: None,
            missing_hash: false,
        };
    }

    if !should_acquire(
        validated_ledger_seq,
        config.ledger_history,
        minimum_online,
        missing,
    ) {
        return HistoryAdvancePlan {
            missing: Some(missing),
            next_state,
            progress: false,
            clear_ledger: None,
            set_full_ledger: None,
            schedule_try_fill: None,
            prefetch: Vec::new(),
            fetch_pack: None,
            missing_hash: false,
        };
    }

    let fetch_result = run_fetch_for_history(
        missing,
        reason,
        config.ledger_fetch_size,
        &next_state.fetch_state,
        hashes,
        ledger_lookup,
        inbound,
        sql,
    );

    next_state.fetch_state = fetch_result.next_state.clone();

    if let Some(cleared_seq) = fetch_result.clear_ledger {
        next_state
            .complete_ledgers
            .erase_interval(range(cleared_seq, cleared_seq));
    }

    let fetch_pack = fetch_result
        .request_fetch_pack
        .as_ref()
        .and_then(|request| {
            let have_hash =
                hashes.get_ledger_hash_for_history(request.missing + 1, request.reason)?;
            if have_hash.is_zero() {
                return None;
            }
            let peer_index = select_fetch_pack_peer(peers, request.missing, clustered)?;
            Some(FetchPackSendPlan {
                peer_index,
                missing: request.missing,
                ledger_hash: have_hash,
                message: make_fetch_pack_request(have_hash),
            })
        });

    if fetch_pack.is_some() {
        next_state.fetch_pack_issued_at = Some(now);
    } else if fetch_result.progress || fetch_result.clear_ledger.is_some() {
        next_state.fetch_pack_issued_at = None;
    }

    HistoryAdvancePlan {
        missing: Some(missing),
        next_state,
        progress: fetch_result.progress,
        clear_ledger: fetch_result.clear_ledger,
        set_full_ledger: fetch_result.set_full_ledger,
        schedule_try_fill: fetch_result.schedule_try_fill,
        prefetch: fetch_result
            .prefetch
            .into_iter()
            .map(|entry| (entry.seq, entry.hash, entry.reason))
            .collect(),
        fetch_pack,
        missing_hash: fetch_result.missing_hash,
    }
}
