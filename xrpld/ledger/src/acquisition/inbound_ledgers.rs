//! `InboundLedgers` support above the landed inbound packet and fetch-pack seams.
//!
//! This ports the local subset from `xrpld/app/ledger/detail/the reference source`:
//! - active-ledger lookup and insertion,
//! - recent-failure bookkeeping,
//! - `gotLedgerData(...)` routing without `Application` / `JobQueue`,
//! - `gotFetchPack()`, `sweep()`, `stop()`, and local fetch-rate tracking,
//! - and stale account-state packet preservation into fetch-pack storage.

use crate::fetch_pack::FetchPackStore;
use crate::{
    FetchPackContainer, InboundLedgerDataType, InboundLedgerJournal, InboundLedgerLocal,
    InboundLedgerPacket, InboundLedgerStore, LedgerConfig,
};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{CacheClock, MonotonicClock};
use protocol::JsonValue;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::tree_node::SHAMapTreeNode;
use std::collections::{BTreeMap, VecDeque};
use std::hash::BuildHasher;
use time::Duration;

pub const INBOUND_LEDGERS_REACQUIRE_INTERVAL: Duration = Duration::minutes(5);
const INBOUND_LEDGERS_SWEEP_INTERVAL: Duration = Duration::minutes(1);
const INBOUND_LEDGERS_FETCH_RATE_WINDOW: Duration = Duration::seconds(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundLedgerRoute {
    ActiveNeedsDispatch,
    ActiveQueued,
    MissingStateStashed,
    MissingIgnored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureRecord {
    seq: u32,
    recorded_at: Duration,
}

#[derive(Debug)]
pub struct InboundLedgersLocal<C = MonotonicClock> {
    clock: C,
    ledgers: BTreeMap<Uint256, InboundLedgerLocal>,
    recent_failures: BTreeMap<Uint256, FailureRecord>,
    fetched_at: VecDeque<Duration>,
    stopping: bool,
}

impl InboundLedgersLocal<MonotonicClock> {
    pub fn new() -> Self {
        Self::with_clock(MonotonicClock::default())
    }
}

impl Default for InboundLedgersLocal<MonotonicClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> InboundLedgersLocal<C>
where
    C: CacheClock,
{
    pub fn with_clock(clock: C) -> Self {
        Self {
            clock,
            ledgers: BTreeMap::new(),
            recent_failures: BTreeMap::new(),
            fetched_at: VecDeque::new(),
            stopping: false,
        }
    }

    pub fn insert(&mut self, inbound: InboundLedgerLocal) -> Option<InboundLedgerLocal> {
        let now = self.clock.now();
        let mut inbound = inbound;
        inbound.touch(now);
        self.ledgers.insert(*inbound.hash().as_uint256(), inbound)
    }

    pub fn find(&self, hash: SHAMapHash) -> Option<&InboundLedgerLocal> {
        self.ledgers.get(hash.as_uint256())
    }

    pub fn find_by_seq(&self, seq: u32) -> Option<&InboundLedgerLocal> {
        self.ledgers.values().find(|inbound| inbound.seq() == seq)
    }

    pub fn find_mut(&mut self, hash: SHAMapHash) -> Option<&mut InboundLedgerLocal> {
        self.ledgers.get_mut(hash.as_uint256())
    }

    pub fn remove(&mut self, hash: SHAMapHash) -> Option<InboundLedgerLocal> {
        self.ledgers.remove(hash.as_uint256())
    }

    pub fn remove_by_seq(&mut self, seq: u32) -> Option<InboundLedgerLocal> {
        let hash = self
            .ledgers
            .iter()
            .find_map(|(hash, inbound)| (inbound.seq() == seq).then_some(*hash))?;
        self.ledgers.remove(&hash)
    }

    pub fn cache_size(&self) -> usize {
        self.ledgers.len()
    }

    pub fn stop(&mut self) {
        self.stopping = true;
        self.ledgers.clear();
        self.recent_failures.clear();
        self.fetched_at.clear();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopping
    }

    pub fn got_ledger_data<FP>(
        &mut self,
        hash: SHAMapHash,
        peer_id: Option<u64>,
        packet: InboundLedgerPacket,
        stale_data_store: &mut FP,
    ) -> InboundLedgerRoute
    where
        FP: FetchPackStore,
    {
        if self.stopping {
            return InboundLedgerRoute::MissingIgnored;
        }

        if let Some(inbound) = self.ledgers.get_mut(hash.as_uint256()) {
            inbound.touch(self.clock.now());
            if inbound.got_data(peer_id, packet) {
                InboundLedgerRoute::ActiveNeedsDispatch
            } else {
                InboundLedgerRoute::ActiveQueued
            }
        } else if packet.packet_type == InboundLedgerDataType::StateNode
            && stash_stale_packet(&packet, stale_data_store)
        {
            InboundLedgerRoute::MissingStateStashed
        } else {
            InboundLedgerRoute::MissingIgnored
        }
    }

    pub fn log_failure(&mut self, hash: SHAMapHash, seq: u32) {
        self.recent_failures.insert(
            *hash.as_uint256(),
            FailureRecord {
                seq,
                recorded_at: self.clock.now(),
            },
        );
    }

    pub fn is_failure(&mut self, hash: SHAMapHash) -> bool {
        self.expire_failures();
        self.recent_failures.contains_key(hash.as_uint256())
    }

    pub fn recent_failure_seq(&mut self, hash: SHAMapHash) -> Option<u32> {
        self.expire_failures();
        self.recent_failures
            .get(hash.as_uint256())
            .map(|record| record.seq)
    }

    pub fn clear_failures(&mut self) {
        self.recent_failures.clear();
        self.ledgers.clear();
    }

    pub fn on_ledger_fetched(&mut self) {
        self.fetched_at.push_back(self.clock.now());
        self.expire_fetch_rate();
    }

    pub fn fetch_rate(&mut self) -> usize {
        self.expire_fetch_rate();
        self.fetched_at.len() * 2
    }

    pub fn sweep(&mut self) -> usize {
        let now = self.clock.now();
        let mut removed = 0usize;

        self.ledgers.retain(|_, inbound| {
            if inbound.last_action() > now {
                inbound.touch(now);
                return true;
            }

            let keep = now - inbound.last_action() < INBOUND_LEDGERS_SWEEP_INTERVAL;
            if !keep {
                removed += 1;
            }
            keep
        });

        self.expire_failures();
        self.expire_fetch_rate();
        removed
    }

    pub fn got_fetch_pack_with_family_and_config<CLOCK, S, FB, F, MR, NS, DB, FP, J>(
        &mut self,
        journal: &J,
        config: &LedgerConfig,
        store: &mut DB,
        fetch_pack: &mut FP,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> usize
    where
        CLOCK: basics::tagged_cache::CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        DB: InboundLedgerStore,
        FP: FetchPackContainer,
        J: InboundLedgerJournal,
    {
        let now = self.clock.now();
        let mut completed = 0usize;

        for inbound in self.ledgers.values_mut() {
            if inbound.is_done() {
                continue;
            }

            let was_done = inbound.is_done();
            if inbound
                .check_local_with_family_and_config(journal, config, store, fetch_pack, family)
                && !was_done
            {
                completed += 1;
                inbound.touch(now);
            }
        }

        completed
    }

    pub fn get_info_with_family<CLOCK, S, FB, F, MR, NS>(
        &mut self,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    ) -> JsonValue
    where
        CLOCK: basics::tagged_cache::CacheClock,
        S: BuildHasher + Clone,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
    {
        let mut root = BTreeMap::new();

        for inbound in self.ledgers.values_mut() {
            let key = if inbound.seq() > 1 {
                inbound.seq().to_string()
            } else {
                inbound.hash().to_string()
            };
            root.insert(key, inbound.get_info_with_family(family));
        }

        for (hash, failure) in &self.recent_failures {
            let key = if failure.seq > 1 {
                failure.seq.to_string()
            } else {
                SHAMapHash::new(*hash).to_string()
            };
            root.insert(
                key,
                JsonValue::Object(BTreeMap::from([(
                    "failed".to_owned(),
                    JsonValue::Bool(true),
                )])),
            );
        }

        JsonValue::Object(root)
    }

    fn expire_failures(&mut self) {
        let now = self.clock.now();
        self.recent_failures
            .retain(|_, record| now - record.recorded_at < INBOUND_LEDGERS_REACQUIRE_INTERVAL);
    }

    fn expire_fetch_rate(&mut self) {
        let now = self.clock.now();
        while self
            .fetched_at
            .front()
            .is_some_and(|recorded_at| now - *recorded_at >= INBOUND_LEDGERS_FETCH_RATE_WINDOW)
        {
            self.fetched_at.pop_front();
        }
    }
}

pub fn stash_stale_packet<FP>(packet: &InboundLedgerPacket, stale_data_store: &mut FP) -> bool
where
    FP: FetchPackStore,
{
    for node in &packet.nodes {
        if node.node_id.is_none() {
            return false;
        }

        let Ok(Some(new_node)) = SHAMapTreeNode::make_from_wire(&node.node_data) else {
            return false;
        };
        let Ok(prefixed) = new_node.serialize_with_prefix() else {
            return false;
        };

        stale_data_store.add_fetch_pack(*new_node.get_hash().as_uint256(), prefixed);
    }

    true
}
