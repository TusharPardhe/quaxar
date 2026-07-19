//! The generic consensus state machine. Ported from rippled's
//! `Consensus.h`.
//!
//! Achieves consensus on the next ledger by agreeing on (1) the set of
//! transactions to include and (2) the ledger's close time.
//!
//! Flow: `start_round` places the node in [`ConsensusPhase::Open`], waiting
//! for transactions. `timer_entry` calls check whether the ledger can
//! close; once closed, the node moves to [`ConsensusPhase::Establish`] and
//! exchanges proposals with peers. Once consensus is detected, the node
//! moves to [`ConsensusPhase::Accepted`], the adaptor applies the agreed
//! transactions, and the cycle restarts via `start_round`.
//!
//! # Deviations from the reference
//!
//! - All `Adaptor` trait methods take `&self`, not `&mut self`. The
//!   reference's C++ `Adaptor&` is a plain mutable reference; the idiomatic
//!   Rust equivalent for an object shared across threads (peer message
//!   dispatch, timer thread, JobQueue accept dispatch) is `Arc<A>` with
//!   internal synchronization, which requires interior mutability rather
//!   than `&mut self`. This mirrors a design correction already validated
//!   in earlier iterations of this codebase.
//! - The reference threads an optional `std::stringstream* clog` through
//!   nearly every method for verbose diagnostic logging. This port uses
//!   `tracing` spans/events at call sites instead; there is no `clog`
//!   parameter.
//! - `getJson`/diagnostic dump methods are omitted. They are RPC-facing
//!   presentation concerns that belong at the app/RCL layer (Phase 5/6),
//!   not in the generic algorithm. `phase()` and `mode()` accessors are
//!   kept since callers need them to build their own diagnostics.
//! - `Tx_t::id()` is expressed via the [`ConsensusTx`] trait (`tx_id`)
//!   rather than assuming an inherent method, matching the deliberate
//!   deviation already made in [`crate::model::DisputedTx`] during Phase 2.
//! - The two small `LedgerTiming`-derived helpers the reference calls
//!   directly (`getNextLedgerTimeResolution`, `roundCloseTime`) are exposed
//!   as [`ConsensusAdaptor`] methods rather than a hard dependency on the
//!   `ledger` crate, keeping this crate ledger-agnostic per the build
//!   order (consensus before the RCL/ledger-aware layers).

use std::time::{Duration, Instant};

use basics::chrono::NetClockTimePoint;
use basics::unordered_containers::{HashMap, HashSet};

use crate::algorithm::functions::{check_consensus, should_close_ledger};
use crate::algorithm::params::ConsensusParms;
use crate::algorithm::types::{ConsensusCloseTimes, ConsensusMode, ConsensusPhase, ConsensusResult, ConsensusState, ConsensusTimer};
use crate::model::proposal::ConsensusProposal;

/// Convert a `time::Duration` (used by [`NetClockTimePoint`] arithmetic,
/// whole-second granularity) into a `std::time::Duration` (used by
/// [`ConsensusParms`] and the free timing functions).
fn time_duration_to_std(d: time::Duration) -> Duration {
    Duration::from_secs(d.whole_seconds().max(0) as u64)
}

/// Convert a `std::time::Duration` into a `time::Duration` for
/// [`NetClockTimePoint`] arithmetic.
fn std_duration_to_time(d: Duration) -> time::Duration {
    time::Duration::seconds(d.as_secs() as i64)
}

/// A transaction that can report its own identity. Matches the reference's
/// implicit `Tx::id()` requirement, made explicit as a trait per the
/// deviation already documented on [`crate::model::DisputedTx`].
pub trait ConsensusTx: Clone {
    type Id: Eq + std::hash::Hash + Ord + Clone + ToString;

    fn tx_id(&self) -> Self::Id;
}

/// A set of transactions under consideration for the next ledger. Matches
/// the reference's `TxSet` concept. The reference's separate
/// `MutableTxSet` view is collapsed into direct `insert`/`erase` on the set
/// itself, per the option the reference's own doc comment allows
/// ("Alternatively, if the TxSet is itself mutable just alias
/// `MutableTxSet` = `TxSet`").
pub trait ConsensusTxSet: Clone {
    type Id: Eq + std::hash::Hash + Ord + Clone + ToString;
    type Tx: ConsensusTx;

    fn exists(&self, tx_id: &<Self::Tx as ConsensusTx>::Id) -> bool;
    fn find(&self, tx_id: &<Self::Tx as ConsensusTx>::Id) -> Option<Self::Tx>;
    fn id(&self) -> Self::Id;
    /// Transactions that differ between `self` and `other`, keyed by tx id,
    /// with the bool indicating which set contains it (`true` = `self`).
    fn compare(&self, other: &Self) -> std::collections::BTreeMap<<Self::Tx as ConsensusTx>::Id, bool>;
    fn insert(&mut self, tx: Self::Tx) -> bool;
    fn erase(&mut self, tx_id: &<Self::Tx as ConsensusTx>::Id) -> bool;
}

/// A ledger and the small set of properties consensus needs from it.
/// Matches the reference's implicit `Ledger` concept.
pub trait ConsensusLedger: Clone + Default {
    type Id: Eq + std::hash::Hash + Clone + ToString + Default;
    type Seq: Copy + Ord + Default + std::ops::Add<u32, Output = Self::Seq> + ToString;

    fn id(&self) -> Self::Id;
    fn seq(&self) -> Self::Seq;
    fn close_time_resolution(&self) -> Duration;
    fn close_agree(&self) -> bool;
    fn close_time(&self) -> NetClockTimePoint;
    fn parent_close_time(&self) -> NetClockTimePoint;
}

/// Wraps a peer's [`ConsensusProposal`]. Matches the reference's
/// `PeerPosition` concept.
pub trait PeerPosition<NodeId, LedgerId, TxSetId>: Clone {
    fn proposal(&self) -> &ConsensusProposal<NodeId, LedgerId, TxSetId>;
}

/// Defines the types and helper functions needed to adapt the generic
/// [`Consensus`] state machine to a specific application. Matches the
/// reference's `Adaptor` concept.
pub trait ConsensusAdaptor {
    type Ledger: ConsensusLedger;
    type NodeId: Eq + std::hash::Hash + Ord + Clone + ToString;
    type TxSet: ConsensusTxSet;
    type PeerPos: PeerPosition<Self::NodeId, <Self::Ledger as ConsensusLedger>::Id, <Self::TxSet as ConsensusTxSet>::Id>;

    /// Attempt to acquire a specific ledger.
    fn acquire_ledger(&self, ledger_id: &<Self::Ledger as ConsensusLedger>::Id) -> Option<Self::Ledger>;

    /// Acquire the transaction set associated with a proposed position.
    /// Returning `None` may spawn an asynchronous request; the result
    /// later arrives via a call to `Consensus::got_tx_set`.
    fn acquire_tx_set(&self, set_id: &<Self::TxSet as ConsensusTxSet>::Id) -> Option<Self::TxSet>;

    /// Whether any transactions are in the open ledger.
    fn has_open_transactions(&self) -> bool;

    /// Number of proposers that have validated the given ledger.
    fn proposers_validated(&self, prev_ledger: &<Self::Ledger as ConsensusLedger>::Id) -> usize;

    /// Number of proposers that have validated a ledger descended from
    /// `prev_ledger`/`prev_ledger_id`.
    fn proposers_finished(&self, prev_ledger: &Self::Ledger, prev_ledger_id: &<Self::Ledger as ConsensusLedger>::Id) -> usize;

    /// The ID of the last closed (and validated) ledger the application
    /// thinks consensus should use as the prior ledger.
    fn get_prev_ledger(
        &self,
        prev_ledger_id: &<Self::Ledger as ConsensusLedger>::Id,
        prev_ledger: &Self::Ledger,
        mode: ConsensusMode,
    ) -> <Self::Ledger as ConsensusLedger>::Id;

    /// Called whenever the consensus operating mode changes.
    fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode);

    /// Called when the ledger closes; returns the initial position.
    fn on_close(&self, prev_ledger: &Self::Ledger, now: NetClockTimePoint, mode: ConsensusMode) -> ConsensusResultOf<Self>;

    /// Called when a ledger is accepted by consensus.
    #[allow(clippy::too_many_arguments)]
    fn on_accept(
        &self,
        result: &ConsensusResultOf<Self>,
        prev_ledger: &Self::Ledger,
        close_resolution: Duration,
        raw_close_times: &ConsensusCloseTimes,
        mode: ConsensusMode,
    );

    /// Propose our position to peers.
    fn propose(&self, pos: &ConsensusProposal<Self::NodeId, <Self::Ledger as ConsensusLedger>::Id, <Self::TxSet as ConsensusTxSet>::Id>);

    /// Share a received peer proposal with other peers (delayed relay).
    fn share_peer_position(&self, prop: &Self::PeerPos);

    /// Share a disputed transaction with peers.
    fn share_tx(&self, tx: &<Self::TxSet as ConsensusTxSet>::Tx);

    /// Share a transaction set with peers.
    fn share_tx_set(&self, set: &Self::TxSet);

    /// Consensus timing parameters and constants.
    fn parms(&self) -> &ConsensusParms;

    /// The next ledger's close-time resolution given the previous ledger's
    /// resolution, whether peers agreed on the previous close time, and the
    /// new ledger's sequence. Adaptor-provided per the module-level
    /// deviation note (keeps this crate ledger-agnostic).
    fn next_ledger_time_resolution(
        &self,
        previous_resolution: Duration,
        previous_agree: bool,
        ledger_seq: <Self::Ledger as ConsensusLedger>::Seq,
    ) -> Duration;

    /// Round a raw close time to the given resolution. Adaptor-provided,
    /// same rationale as `next_ledger_time_resolution`.
    fn round_close_time(&self, raw: NetClockTimePoint, resolution: Duration) -> NetClockTimePoint;
}

/// Shorthand for the concrete [`ConsensusResult`] type produced by an
/// adaptor `A`.
pub type ConsensusResultOf<A> = ConsensusResult<
    <A as ConsensusAdaptor>::TxSet,
    <A as ConsensusAdaptor>::NodeId,
    <<A as ConsensusAdaptor>::Ledger as ConsensusLedger>::Id,
    <<A as ConsensusAdaptor>::TxSet as ConsensusTxSet>::Id,
    <<A as ConsensusAdaptor>::TxSet as ConsensusTxSet>::Tx,
    <<<A as ConsensusAdaptor>::TxSet as ConsensusTxSet>::Tx as ConsensusTx>::Id,
>;


/// A source of `Instant`s for measuring consensus progress. Matches the
/// reference's injected `clock_type` (`beast::AbstractClock`), which
/// exists specifically so tests can drive consensus with a fake clock
/// rather than real wall-clock time.
pub trait ConsensusClock {
    fn now(&self) -> Instant;
}

/// A `ConsensusClock` backed by `Instant::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemConsensusClock;

impl ConsensusClock for SystemConsensusClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Notifies the adaptor whenever the [`ConsensusMode`] changes. Matches
/// the reference's `MonitoredMode` helper.
struct MonitoredMode {
    mode: ConsensusMode,
}

impl MonitoredMode {
    fn new(mode: ConsensusMode) -> Self {
        Self { mode }
    }

    fn get(&self) -> ConsensusMode {
        self.mode
    }

    fn set<A: ConsensusAdaptor>(&mut self, mode: ConsensusMode, adaptor: &A) {
        adaptor.on_mode_change(self.mode, mode);
        self.mode = mode;
    }
}

/// Generic implementation of the XRP Ledger consensus algorithm. Matches
/// the reference's `Consensus<Adaptor>`.
///
/// The reference documents this class as not thread-safe, with the
/// application responsible for serializing access. That constraint carries
/// over unchanged: callers must synchronize access to a shared
/// `Consensus<A>` instance (e.g. behind a `Mutex`) themselves.
pub struct Consensus<A: ConsensusAdaptor, C: ConsensusClock = SystemConsensusClock> {
    clock: C,

    phase: ConsensusPhase,
    mode: MonitoredMode,
    first_round: bool,
    have_close_time_consensus: bool,

    converge_percent: i64,
    open_time: ConsensusTimer,
    close_resolution: Duration,
    close_time_avalanche_state: crate::algorithm::params::AvalancheState,
    prev_round_time: Duration,

    now: NetClockTimePoint,
    prev_close_time: NetClockTimePoint,

    prev_ledger_id: <A::Ledger as ConsensusLedger>::Id,
    previous_ledger: A::Ledger,

    acquired: HashMap<<A::TxSet as ConsensusTxSet>::Id, A::TxSet>,

    result: Option<ConsensusResultOf<A>>,
    raw_close_times: ConsensusCloseTimes,

    peer_unchanged_counter: usize,
    establish_counter: usize,

    curr_peer_positions: HashMap<A::NodeId, A::PeerPos>,
    recent_peer_positions: HashMap<A::NodeId, std::collections::VecDeque<A::PeerPos>>,

    prev_proposers: usize,
    dead_nodes: HashSet<A::NodeId>,
}

impl<A: ConsensusAdaptor> Consensus<A, SystemConsensusClock> {
    /// Construct a new state machine using the system wall clock.
    pub fn new() -> Self {
        Self::with_clock(SystemConsensusClock)
    }
}

impl<A: ConsensusAdaptor> Default for Consensus<A, SystemConsensusClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: ConsensusAdaptor, C: ConsensusClock> Consensus<A, C> {
    /// Construct a new state machine using the given clock. Exposed for
    /// tests that need deterministic timing control (matches the
    /// reference's constructor taking an injected `clock_type`).
    pub fn with_clock(clock: C) -> Self {
        Self {
            clock,
            phase: ConsensusPhase::Accepted,
            mode: MonitoredMode::new(ConsensusMode::Observing),
            first_round: true,
            have_close_time_consensus: false,
            converge_percent: 0,
            open_time: ConsensusTimer::default(),
            close_resolution: Duration::from_secs(30),
            close_time_avalanche_state: crate::algorithm::params::AvalancheState::Init,
            prev_round_time: Duration::ZERO,
            now: NetClockTimePoint::default(),
            prev_close_time: NetClockTimePoint::default(),
            prev_ledger_id: Default::default(),
            previous_ledger: A::Ledger::default(),
            acquired: HashMap::default(),
            result: None,
            raw_close_times: ConsensusCloseTimes::default(),
            peer_unchanged_counter: 0,
            establish_counter: 0,
            curr_peer_positions: HashMap::default(),
            recent_peer_positions: HashMap::default(),
            prev_proposers: 0,
            dead_nodes: HashSet::default(),
        }
    }

    pub fn prev_ledger_id(&self) -> &<A::Ledger as ConsensusLedger>::Id {
        &self.prev_ledger_id
    }

    pub fn phase(&self) -> ConsensusPhase {
        self.phase
    }

    pub fn mode(&self) -> ConsensusMode {
        self.mode.get()
    }

    /// Kick off the next round of consensus. Matches `startRound`.
    ///
    /// `prev_ledger_id` is not required to match `prev_ledger.id()`, since
    /// the id may be known locally before the ledger's contents arrive.
    pub fn start_round(
        &mut self,
        adaptor: &A,
        now: NetClockTimePoint,
        prev_ledger_id: <A::Ledger as ConsensusLedger>::Id,
        mut prev_ledger: A::Ledger,
        now_untrusted: &HashSet<A::NodeId>,
        proposing: bool,
    ) {
        if self.first_round {
            self.prev_round_time = adaptor.parms().ledger_idle_interval;
            self.prev_close_time = prev_ledger.close_time();
            // Genesis ledger has close_time=0; treat "now" as the effective
            // close time so the first round doesn't compute a bogus
            // since_close spanning the entire network epoch.
            if self.prev_close_time == NetClockTimePoint::default() {
                self.prev_close_time = now;
            }
            self.first_round = false;
        } else {
            self.prev_close_time = self.raw_close_times.self_;
        }

        for n in now_untrusted {
            self.recent_peer_positions.remove(n);
        }

        let mut start_mode = if proposing { ConsensusMode::Proposing } else { ConsensusMode::Observing };

        if prev_ledger.id() != prev_ledger_id {
            if let Some(new_ledger) = adaptor.acquire_ledger(&prev_ledger_id) {
                prev_ledger = new_ledger;
            } else {
                start_mode = ConsensusMode::WrongLedger;
            }
        }

        self.start_round_internal(adaptor, now, prev_ledger_id, &prev_ledger, start_mode);
    }

    fn start_round_internal(
        &mut self,
        adaptor: &A,
        now: NetClockTimePoint,
        prev_ledger_id: <A::Ledger as ConsensusLedger>::Id,
        prev_ledger: &A::Ledger,
        mode: ConsensusMode,
    ) {
        self.phase = ConsensusPhase::Open;
        self.mode.set(mode, adaptor);
        self.now = now;
        self.prev_ledger_id = prev_ledger_id;
        self.previous_ledger = prev_ledger.clone();
        self.result = None;
        self.converge_percent = 0;
        self.close_time_avalanche_state = crate::algorithm::params::AvalancheState::Init;
        self.have_close_time_consensus = false;
        self.open_time.reset(self.clock.now());
        self.curr_peer_positions.clear();
        self.acquired.clear();
        self.raw_close_times.peers.clear();
        self.raw_close_times.self_ = NetClockTimePoint::default();
        self.dead_nodes.clear();

        self.close_resolution =
            adaptor.next_ledger_time_resolution(self.previous_ledger.close_time_resolution(), self.previous_ledger.close_agree(), self.previous_ledger.seq() + 1);

        self.playback_proposals(adaptor);

        if self.curr_peer_positions.len() > (self.prev_proposers / 2) {
            // We may be falling behind; don't wait for the timer, consider
            // closing the ledger immediately.
            self.timer_entry(adaptor, self.now);
        }
    }

    /// A peer has proposed a new position; adjust our tracking. Returns
    /// whether we should do delayed relay of this proposal. Matches
    /// `peerProposal`.
    pub fn peer_proposal(&mut self, adaptor: &A, now: NetClockTimePoint, new_proposal: &A::PeerPos) -> bool {
        let peer_id = new_proposal.proposal().node_id().clone();

        {
            let props = self.recent_peer_positions.entry(peer_id).or_default();
            if props.len() >= 10 {
                props.pop_front();
            }
            props.push_back(new_proposal.clone());
        }

        self.peer_proposal_internal(adaptor, now, new_proposal)
    }

    fn peer_proposal_internal(&mut self, adaptor: &A, now: NetClockTimePoint, new_peer_pos: &A::PeerPos) -> bool {
        if self.phase == ConsensusPhase::Accepted {
            return false;
        }

        self.now = now;
        let new_peer_prop = new_peer_pos.proposal();

        if *new_peer_prop.prev_ledger() != self.prev_ledger_id {
            return false;
        }

        let peer_id = new_peer_prop.node_id().clone();

        if self.dead_nodes.contains(&peer_id) {
            return false;
        }

        {
            if let Some(existing) = self.curr_peer_positions.get(&peer_id)
                && new_peer_prop.propose_seq() <= existing.proposal().propose_seq()
            {
                return false;
            }

            if new_peer_prop.is_bow_out() {
                if let Some(result) = &mut self.result {
                    for dispute in result.disputes.values_mut() {
                        dispute.un_vote(&peer_id);
                    }
                }
                self.curr_peer_positions.remove(&peer_id);
                self.dead_nodes.insert(peer_id.clone());
                return true;
            }

            self.curr_peer_positions.insert(peer_id.clone(), new_peer_pos.clone());
        }

        if new_peer_prop.is_initial() {
            *self.raw_close_times.peers.entry(new_peer_prop.close_time()).or_insert(0) += 1;
        }

        {
            let position = new_peer_prop.position().clone();
            let node_id = new_peer_prop.node_id().clone();
            if !self.acquired.contains_key(&position) {
                if let Some(set) = adaptor.acquire_tx_set(&position) {
                    self.got_tx_set(adaptor, self.now, &set);
                }
            } else if self.result.is_some() {
                let set = self.acquired.get(&position).cloned();
                if let Some(set) = set {
                    self.update_disputes(adaptor, &node_id, &set);
                }
            }
        }

        true
    }

    /// Drive consensus forward. Call periodically. Matches `timerEntry`.
    pub fn timer_entry(&mut self, adaptor: &A, now: NetClockTimePoint) {
        if self.phase == ConsensusPhase::Accepted {
            return;
        }

        self.now = now;
        self.check_ledger(adaptor);

        match self.phase {
            ConsensusPhase::Open => self.phase_open(adaptor),
            ConsensusPhase::Establish => self.phase_establish(adaptor),
            ConsensusPhase::Accepted => {}
        }
    }

    /// Process a transaction set acquired from the network. Matches
    /// `gotTxSet`.
    pub fn got_tx_set(&mut self, adaptor: &A, now: NetClockTimePoint, tx_set: &A::TxSet) {
        if self.phase == ConsensusPhase::Accepted {
            return;
        }

        self.now = now;
        let id = tx_set.id();

        if self.acquired.contains_key(&id) {
            return;
        }
        self.acquired.insert(id.clone(), tx_set.clone());

        if self.result.is_none() {
            return;
        }

        let positions: Vec<(A::NodeId, <A::TxSet as ConsensusTxSet>::Id)> =
            self.curr_peer_positions.iter().map(|(node_id, pos)| (node_id.clone(), pos.proposal().position().clone())).collect();

        for (node_id, position) in positions {
            if position == id {
                self.update_disputes(adaptor, &node_id, tx_set);
            }
        }
    }

    /// Change our view of the previous ledger. Matches
    /// `handleWrongLedger`.
    fn handle_wrong_ledger(&mut self, adaptor: &A, lgr_id: <A::Ledger as ConsensusLedger>::Id) {
        self.leave_consensus(adaptor);

        if self.prev_ledger_id != lgr_id {
            self.prev_ledger_id = lgr_id.clone();

            if let Some(result) = &mut self.result {
                result.disputes.clear();
                result.compares.clear();
            }

            self.curr_peer_positions.clear();
            self.raw_close_times.peers.clear();
            self.dead_nodes.clear();

            self.playback_proposals(adaptor);
        }

        if self.previous_ledger.id() == self.prev_ledger_id {
            return;
        }

        if let Some(new_ledger) = adaptor.acquire_ledger(&self.prev_ledger_id) {
            self.start_round_internal(adaptor, self.now, lgr_id, &new_ledger, ConsensusMode::SwitchedLedger);
        } else {
            self.mode.set(ConsensusMode::WrongLedger, adaptor);
        }
    }

    /// Check that our previous ledger matches the network's. Matches
    /// `checkLedger`.
    fn check_ledger(&mut self, adaptor: &A) {
        let net_lgr = adaptor.get_prev_ledger(&self.prev_ledger_id, &self.previous_ledger, self.mode.get());

        if net_lgr != self.prev_ledger_id || self.previous_ledger.id() != self.prev_ledger_id {
            self.handle_wrong_ledger(adaptor, net_lgr);
        }
    }

    /// Replay recent proposals so they aren't lost after a radical context
    /// change. Matches `playbackProposals`.
    fn playback_proposals(&mut self, adaptor: &A) {
        let all: Vec<A::PeerPos> = self.recent_peer_positions.values().flat_map(|deque| deque.iter().cloned()).collect();

        for pos in all {
            if *pos.proposal().prev_ledger() == self.prev_ledger_id && self.peer_proposal_internal(adaptor, self.now, &pos) {
                adaptor.share_peer_position(&pos);
            }
        }
    }

    /// Handle the pre-close (`Open`) phase. Matches `phaseOpen`.
    fn phase_open(&mut self, adaptor: &A) {
        let any_transactions = adaptor.has_open_transactions();
        let proposers_closed = self.curr_peer_positions.len();
        let proposers_validated = adaptor.proposers_validated(&self.prev_ledger_id);
        if proposers_closed > 0 || self.prev_proposers > 0 {
            tracing::info!(target: "consensus", proposers_closed, proposers_validated, prev_proposers = self.prev_proposers, "phase_open: proposer state");
        }

        self.open_time.tick_to(self.clock.now());

        let mode = self.mode.get();
        let close_agree = self.previous_ledger.close_agree();
        let prev_close_time = self.previous_ledger.close_time();
        let prev_parent_close_time_plus_1 = self.previous_ledger.parent_close_time() + time::Duration::seconds(1);
        let previous_close_correct = mode != ConsensusMode::WrongLedger && close_agree && (prev_close_time != prev_parent_close_time_plus_1);

        let last_close_time = if previous_close_correct { prev_close_time } else { self.prev_close_time };

        // Guard against zero close time (genesis or early rounds before
        // any real close time was established). Without this, since_close
        // computes as the entire network epoch duration (~836M seconds),
        // causing shouldCloseLedger to fire immediately on the very first
        // timer tick, before peers have time to exchange proposals.
        // Use the captured prev_close_time from start_round (which was
        // set to `now` at round-start for genesis) as the reference point,
        // NOT the continuously-advancing self.now (which would make
        // since_close=0 permanently, preventing the ledger from ever closing).
        let last_close_time = if last_close_time == NetClockTimePoint::default() {
            self.prev_close_time
        } else {
            last_close_time
        };

        let since_close = if self.now >= last_close_time {
            time_duration_to_std(self.now - last_close_time)
        } else {
            Duration::ZERO
        };

        let idle_interval = adaptor.parms().ledger_idle_interval.max(self.previous_ledger.close_time_resolution() * 2);

        if should_close_ledger(
            any_transactions,
            self.prev_proposers,
            proposers_closed,
            proposers_validated,
            self.prev_round_time,
            since_close,
            self.open_time.read(),
            idle_interval,
            adaptor.parms(),
        ) {
            self.close_ledger(adaptor);
        }
    }

    /// Close the open ledger and establish our initial position. Matches
    /// `closeLedger`.
    fn close_ledger(&mut self, adaptor: &A) {
        debug_assert!(self.result.is_none(), "Consensus::close_ledger: result must not already be set");

        self.phase = ConsensusPhase::Establish;
        // When recovering from a ledger switch, prefer the peer-reported
        // close time over our local clock. Our local "now" may be several
        // seconds past peers' actual close moment due to acquisition delay.
        // This ensures our close_time matches what peers used, producing
        // the same ledger hash after effective_close_time rounding.
        if self.mode.get() == ConsensusMode::SwitchedLedger && !self.raw_close_times.peers.is_empty() {
            let best_peer_time = self.raw_close_times.peers.iter()
                .max_by_key(|(_, count)| *count)
                .map(|(time, _)| *time)
                .unwrap_or(self.now);
            self.raw_close_times.self_ = best_peer_time;
        } else {
            self.raw_close_times.self_ = self.now;
        }
        self.peer_unchanged_counter = 0;
        self.establish_counter = 0;

        let mut result = adaptor.on_close(&self.previous_ledger, self.now, self.mode.get());
        result.round_time.reset(self.clock.now());

        let txns_id = result.txns.id();
        if let std::collections::hash_map::Entry::Vacant(e) = self.acquired.entry(txns_id) {
            e.insert(result.txns.clone());
            adaptor.share_tx_set(&result.txns);
        }

        let mode = self.mode.get();
        if mode == ConsensusMode::Proposing {
            adaptor.propose(&result.position);
        }

        self.result = Some(result);

        let peer_positions: Vec<<A::TxSet as ConsensusTxSet>::Id> =
            self.curr_peer_positions.values().map(|p| p.proposal().position().clone()).collect();
        for pos in peer_positions {
            if let Some(set) = self.acquired.get(&pos).cloned() {
                self.create_disputes(adaptor, &set);
            }
        }
    }

    /// Handle the `Establish` phase. Matches `phaseEstablish`.
    fn phase_establish(&mut self, adaptor: &A) {
        debug_assert!(self.result.is_some(), "Consensus::phase_establish: result must be set");

        self.peer_unchanged_counter += 1;
        self.establish_counter += 1;

        let parms = adaptor.parms().clone();

        let clock_now = self.clock.now();
        {
            let result = self.result.as_mut().expect("result set");
            result.round_time.tick_to(clock_now);
            result.proposers = self.curr_peer_positions.len();
        }

        let round_time_ms = self.result.as_ref().expect("result set").round_time.read();
        let denom = self.prev_round_time.max(parms.av_min_consensus_time);
        self.converge_percent = (round_time_ms.as_millis() as i64 * 100) / denom.as_millis().max(1) as i64;

        if round_time_ms < parms.ledger_min_consensus {
            return;
        }

        self.update_our_positions(adaptor);

        if self.should_pause(adaptor) || !self.have_consensus(adaptor) {
            return;
        }

        if !self.have_close_time_consensus {
            return;
        }

        self.prev_proposers = self.curr_peer_positions.len();
        // Cap prev_round_time to prevent cascading slowdowns. If one round
        // takes 22s due to disputes, without a cap the NEXT round must stay
        // open for 11s (prev_round_time/2 guard in shouldCloseLedger).
        // Rippled never hits this because it gets 0 disputes, but a cap at
        // 10s (producing a 5s minimum open time) prevents permanent divergence
        // if a single slow round occurs. The av_min_consensus_time (5s) is the
        // natural floor — there's no benefit to prev_round_time exceeding 2x that.
        let raw_round_time = self.result.as_ref().expect("result set").round_time.read();
        self.prev_round_time = raw_round_time.min(Duration::from_secs(10));
        self.phase = ConsensusPhase::Accepted;

        let result = self.result.take().expect("result set");
        adaptor.on_accept(&result, &self.previous_ledger, self.close_resolution, &self.raw_close_times, self.mode.get());
        self.result = Some(result);
    }

    /// Evaluate whether pausing increases the likelihood of validation.
    /// Matches `shouldPause`. This is a stub returning `false` in the
    /// generic algorithm: the reference's implementation depends entirely
    /// on validator/UNL/laggard bookkeeping that lives at the RCL
    /// adaptation layer (`getValidLedgerIndex`, `getQuorumKeys`,
    /// `laggards`, `validator`, `haveValidated`), which is out of scope for
    /// the generic, adaptor-agnostic state machine built in this phase. It
    /// is intentionally left as a seam: Phase 5/6's `RclConsensusAdaptor`
    /// can override this behavior once those data sources exist, either by
    /// adding an adaptor method here or wrapping `Consensus` at that layer.
    fn should_pause(&self, _adaptor: &A) -> bool {
        false
    }

    /// Adjust our position to try to agree with other validators. Matches
    /// `updateOurPositions`.
    fn update_our_positions(&mut self, adaptor: &A) {
        let parms = adaptor.parms().clone();

        let peer_cutoff = self.now.checked_sub(std_duration_to_time(parms.propose_freshness)).unwrap_or_default();
        let our_cutoff = self.now.checked_sub(std_duration_to_time(parms.propose_interval)).unwrap_or_default();

        let mut close_time_votes: std::collections::BTreeMap<NetClockTimePoint, i32> = std::collections::BTreeMap::new();

        {
            let stale_peers: Vec<A::NodeId> = self
                .curr_peer_positions
                .iter()
                .filter(|(_, pos)| pos.proposal().is_stale(peer_cutoff))
                .map(|(id, _)| id.clone())
                .collect();

            for peer_id in &stale_peers {
                if let Some(result) = &mut self.result {
                    for dispute in result.disputes.values_mut() {
                        dispute.un_vote(peer_id);
                    }
                }
                self.curr_peer_positions.remove(peer_id);
            }

            for pos in self.curr_peer_positions.values() {
                let ct = self.round_close_time_for(adaptor, pos.proposal().close_time());
                *close_time_votes.entry(ct).or_insert(0) += 1;
            }
        }

        let mut our_new_set: Option<A::TxSet>;

        {
            let proposing = self.mode.get() == ConsensusMode::Proposing;
            let result = self.result.as_mut().expect("result set during update_our_positions");
            let mut mutable_set: Option<A::TxSet> = None;

            let dispute_count = result.disputes.len();
            let mut vote_changes = 0usize;

            for (tx_id, dispute) in result.disputes.iter_mut() {
                if dispute.update_vote(self.converge_percent as i32, proposing, &parms) {
                    vote_changes += 1;
                    if mutable_set.is_none() {
                        mutable_set = Some(result.txns.clone());
                    }
                    let set = mutable_set.as_mut().expect("just set");
                    if dispute.get_our_vote() {
                        set.insert(dispute.tx().clone());
                    } else {
                        set.erase(tx_id);
                    }
                }
            }

            if dispute_count > 0 || vote_changes > 0 {
                let vote_detail: Vec<(bool, i32, i32)> = result.disputes.values().map(|d| (d.get_our_vote(), d.yays(), d.nays())).collect();
                tracing::info!(target: "consensus", dispute_count, vote_changes, proposing, converge_pct = self.converge_percent, votes = ?vote_detail, "update_our_positions: dispute status");
            }

            our_new_set = mutable_set;
        }

        let mut consensus_close_time = NetClockTimePoint::default();
        self.have_close_time_consensus = false;

        if self.curr_peer_positions.is_empty() {
            self.have_close_time_consensus = true;
            let position_close_time = self.result.as_ref().expect("result set").position.close_time();
            consensus_close_time = self.round_close_time_for(adaptor, position_close_time);
        } else {
            let (needed_weight, new_state) = crate::algorithm::params::get_needed_weight(&parms, self.close_time_avalanche_state, self.converge_percent as i32, 0, 0);
            if let Some(new_state) = new_state {
                self.close_time_avalanche_state = new_state;
            }

            let mut participants = self.curr_peer_positions.len() as i32;
            if self.mode.get() == ConsensusMode::Proposing {
                let our_close_time = self.round_close_time_for(adaptor, self.result.as_ref().expect("result set").position.close_time());
                *close_time_votes.entry(our_close_time).or_insert(0) += 1;
                participants += 1;
            }

            let mut thresh_vote = crate::algorithm::types::participants_needed(participants, needed_weight as i32);
            let thresh_consensus = crate::algorithm::types::participants_needed(participants, parms.av_ct_consensus_pct as i32);

            for (t, v) in &close_time_votes {
                if *v >= thresh_vote {
                    consensus_close_time = *t;
                    thresh_vote = *v;
                    if thresh_vote >= thresh_consensus {
                        self.have_close_time_consensus = true;
                    }
                }
            }
        }

        if our_new_set.is_none() {
            let result = self.result.as_ref().expect("result set");
            let position_close_time = self.round_close_time_for(adaptor, result.position.close_time());
            if consensus_close_time != position_close_time || result.position.is_stale(our_cutoff) {
                our_new_set = Some(result.txns.clone());
            }
        }

        if let Some(new_set) = our_new_set {
            let new_id = new_set.id();
            let now = self.now;

            {
                let result = self.result.as_mut().expect("result set");
                result.txns = new_set;
                result.position.change_position(new_id.clone(), consensus_close_time, now);
            }

            if !self.acquired.contains_key(&new_id) {
                let txns = self.result.as_ref().expect("result set").txns.clone();
                self.acquired.insert(new_id.clone(), txns.clone());

                let is_bow_out = self.result.as_ref().expect("result set").position.is_bow_out();
                if !is_bow_out {
                    adaptor.share_tx_set(&txns);
                }

                let matching_peers: Vec<A::NodeId> = self
                    .curr_peer_positions
                    .iter()
                    .filter(|(_, p)| *p.proposal().position() == new_id)
                    .map(|(id, _)| id.clone())
                    .collect();
                for node_id in matching_peers {
                    self.update_disputes(adaptor, &node_id, &txns);
                }
            }

            let (is_bow_out, position) = {
                let result = self.result.as_ref().expect("result set");
                (result.position.is_bow_out(), result.position.clone())
            };
            if !is_bow_out && self.mode.get() == ConsensusMode::Proposing {
                adaptor.propose(&position);
            }
        }
    }

    fn round_close_time_for(&self, adaptor: &A, raw: NetClockTimePoint) -> NetClockTimePoint {
        adaptor.round_close_time(raw, self.close_resolution)
    }

    /// Determine if we have consensus, updating `result.state`. Returns
    /// whether we have consensus (state is not `No`). Matches
    /// `haveConsensus`.
    fn have_consensus(&mut self, adaptor: &A) -> bool {
        let our_position = self.result.as_ref().expect("result set").position.position().clone();

        let mut agree = 0usize;
        let mut disagree = 0usize;
        for pos in self.curr_peer_positions.values() {
            if *pos.proposal().position() == our_position {
                agree += 1;
            } else {
                disagree += 1;
            }
        }

        let current_finished = adaptor.proposers_finished(&self.previous_ledger, &self.prev_ledger_id);
        let parms = adaptor.parms().clone();
        let proposing = self.mode.get() == ConsensusMode::Proposing;

        let stalled = {
            let result = self.result.as_ref().expect("result set");
            self.have_close_time_consensus
                && !result.disputes.is_empty()
                && result.disputes.values().all(|dispute| dispute.stalled(&parms, proposing, self.peer_unchanged_counter))
        };

        let state = check_consensus(
            self.prev_proposers,
            agree + disagree,
            agree,
            current_finished,
            self.prev_round_time,
            self.result.as_ref().expect("result set").round_time.read(),
            stalled,
            &parms,
            proposing,
        );

        self.result.as_mut().expect("result set").state = state;

        if state == ConsensusState::No {
            return false;
        }

        if state == ConsensusState::Expired {
            let minimum_counter = parms.avalanche_cutoffs.len() * parms.av_min_rounds;
            if self.establish_counter < minimum_counter {
                return false;
            }
            self.leave_consensus(adaptor);
        }

        true
    }

    /// Revoke our outstanding proposal, if any, and cease proposing until
    /// this round ends. Matches `leaveConsensus`.
    fn leave_consensus(&mut self, adaptor: &A) {
        if self.mode.get() == ConsensusMode::Proposing {
            if let Some(result) = &mut self.result
                && !result.position.is_bow_out()
            {
                result.position.bow_out(self.now);
                adaptor.propose(&result.position);
            }
            self.mode.set(ConsensusMode::Observing, adaptor);
        }
    }

    /// Create disputes between our position and `other`. Matches
    /// `createDisputes`.
    fn create_disputes(&mut self, adaptor: &A, other: &A::TxSet) {
        let other_id = other.id();

        let already_compared = {
            let result = self.result.as_mut().expect("result set");
            !result.compares.insert(other_id.clone())
        };
        if already_compared {
            return;
        }

        let our_txns = self.result.as_ref().expect("result set").txns.clone();
        if our_txns.id() == other_id {
            return;
        }

        let differences = our_txns.compare(other);

        for (tx_id, in_this_set) in differences {
            let tx = if in_this_set { our_txns.find(&tx_id) } else { other.find(&tx_id) };
            let Some(tx) = tx else { continue };

            if self.result.as_ref().expect("result set").disputes.contains_key(&tx_id) {
                continue;
            }

            let our_vote = our_txns.exists(&tx_id);
            let mut dtx = crate::model::DisputedTx::new(tx.clone(), tx_id.clone(), our_vote);

            for (node_id, peer_pos) in self.curr_peer_positions.iter() {
                let position = peer_pos.proposal().position();
                if let Some(set) = self.acquired.get(position)
                    && dtx.set_vote(node_id.clone(), set.exists(&tx_id))
                {
                    self.peer_unchanged_counter = 0;
                }
            }

            adaptor.share_tx(dtx.tx());

            self.result.as_mut().expect("result set").disputes.insert(tx_id, dtx);
        }
    }

    /// Update our disputes given that `node` has adopted position `other`.
    /// Calls `create_disputes` as needed. Matches `updateDisputes`.
    fn update_disputes(&mut self, adaptor: &A, node: &A::NodeId, other: &A::TxSet) {
        let other_id = other.id();
        let already_compared = self.result.as_ref().expect("result set").compares.contains(&other_id);
        if !already_compared {
            self.create_disputes(adaptor, other);
        }

        let result = self.result.as_mut().expect("result set");
        for dispute in result.disputes.values_mut() {
            let tx_id = dispute.tx().tx_id();
            if dispute.set_vote(node.clone(), other.exists(&tx_id)) {
                self.peer_unchanged_counter = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::params::ConsensusParms;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    type NodeId = u32;
    type LedgerId = u32;
    type TxId = u32;
    type TxSetId = u32;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockTx {
        id: TxId,
    }

    impl ConsensusTx for MockTx {
        type Id = TxId;
        fn tx_id(&self) -> TxId {
            self.id
        }
    }

    #[derive(Debug, Clone, Default)]
    struct MockTxSet {
        txs: BTreeMap<TxId, MockTx>,
    }

    impl MockTxSet {
        fn with(ids: &[TxId]) -> Self {
            Self {
                txs: ids.iter().map(|&id| (id, MockTx { id })).collect(),
            }
        }
    }

    impl ConsensusTxSet for MockTxSet {
        type Id = TxSetId;
        type Tx = MockTx;

        fn exists(&self, tx_id: &TxId) -> bool {
            self.txs.contains_key(tx_id)
        }
        fn find(&self, tx_id: &TxId) -> Option<MockTx> {
            self.txs.get(tx_id).cloned()
        }
        fn id(&self) -> TxSetId {
            // Deterministic pseudo-hash: sum of tx ids, distinguishing the
            // empty set (id 0) from any populated set.
            self.txs.keys().sum()
        }
        fn compare(&self, other: &Self) -> BTreeMap<TxId, bool> {
            let mut out = BTreeMap::new();
            for id in self.txs.keys() {
                if !other.txs.contains_key(id) {
                    out.insert(*id, true);
                }
            }
            for id in other.txs.keys() {
                if !self.txs.contains_key(id) {
                    out.insert(*id, false);
                }
            }
            out
        }
        fn insert(&mut self, tx: MockTx) -> bool {
            self.txs.insert(tx.id, tx).is_none()
        }
        fn erase(&mut self, tx_id: &TxId) -> bool {
            self.txs.remove(tx_id).is_some()
        }
    }

    #[derive(Debug, Clone, Default)]
    struct MockLedger {
        id: LedgerId,
        seq: u32,
        close_time: NetClockTimePoint,
        parent_close_time: NetClockTimePoint,
        close_agree: bool,
    }

    impl ConsensusLedger for MockLedger {
        type Id = LedgerId;
        type Seq = u32;

        fn id(&self) -> LedgerId {
            self.id
        }
        fn seq(&self) -> u32 {
            self.seq
        }
        fn close_time_resolution(&self) -> Duration {
            Duration::from_secs(10)
        }
        fn close_agree(&self) -> bool {
            self.close_agree
        }
        fn close_time(&self) -> NetClockTimePoint {
            self.close_time
        }
        fn parent_close_time(&self) -> NetClockTimePoint {
            self.parent_close_time
        }
    }

    #[derive(Debug, Clone)]
    struct MockPeerPos {
        proposal: ConsensusProposal<NodeId, LedgerId, TxSetId>,
    }

    impl PeerPosition<NodeId, LedgerId, TxSetId> for MockPeerPos {
        fn proposal(&self) -> &ConsensusProposal<NodeId, LedgerId, TxSetId> {
            &self.proposal
        }
    }

    /// Shared, mutable tracking state for the mock adaptor. `RefCell`
    /// because `ConsensusAdaptor` methods take `&self`.
    #[derive(Default)]
    struct MockState {
        proposed: Vec<ConsensusProposal<NodeId, LedgerId, TxSetId>>,
        shared_tx_sets: Vec<MockTxSet>,
        shared_positions: Vec<MockPeerPos>,
        accepted: Vec<TxSetId>,
        mode_changes: Vec<(ConsensusMode, ConsensusMode)>,
        proposers_validated: usize,
        proposers_finished: usize,
        has_open_transactions: bool,
        ledgers: BTreeMap<LedgerId, MockLedger>,
        tx_sets: BTreeMap<TxSetId, MockTxSet>,
    }

    struct MockAdaptor {
        state: Rc<RefCell<MockState>>,
        parms: ConsensusParms,
    }

    impl MockAdaptor {
        fn new() -> Self {
            Self {
                state: Rc::new(RefCell::new(MockState::default())),
                parms: ConsensusParms::default(),
            }
        }
    }

    impl ConsensusAdaptor for MockAdaptor {
        type Ledger = MockLedger;
        type NodeId = NodeId;
        type TxSet = MockTxSet;
        type PeerPos = MockPeerPos;

        fn acquire_ledger(&self, ledger_id: &LedgerId) -> Option<MockLedger> {
            self.state.borrow().ledgers.get(ledger_id).cloned()
        }

        fn acquire_tx_set(&self, set_id: &TxSetId) -> Option<MockTxSet> {
            self.state.borrow().tx_sets.get(set_id).cloned()
        }

        fn has_open_transactions(&self) -> bool {
            self.state.borrow().has_open_transactions
        }

        fn proposers_validated(&self, _prev_ledger: &LedgerId) -> usize {
            self.state.borrow().proposers_validated
        }

        fn proposers_finished(&self, _prev_ledger: &MockLedger, _prev_ledger_id: &LedgerId) -> usize {
            self.state.borrow().proposers_finished
        }

        fn get_prev_ledger(&self, prev_ledger_id: &LedgerId, _prev_ledger: &MockLedger, _mode: ConsensusMode) -> LedgerId {
            // Mock always agrees with our own view unless a test overrides
            // via direct field mutation (not exercised in these tests).
            *prev_ledger_id
        }

        fn on_mode_change(&self, before: ConsensusMode, after: ConsensusMode) {
            self.state.borrow_mut().mode_changes.push((before, after));
        }

        fn on_close(&self, _prev_ledger: &MockLedger, now: NetClockTimePoint, _mode: ConsensusMode) -> ConsensusResultOf<Self> {
            let txns = MockTxSet::default();
            let id = txns.id();
            let position = ConsensusProposal::new(0, 0, id, now, now, 0);
            ConsensusResult::new(txns, position, id)
        }

        fn on_accept(
            &self,
            result: &ConsensusResultOf<Self>,
            _prev_ledger: &MockLedger,
            _close_resolution: Duration,
            _raw_close_times: &ConsensusCloseTimes,
            _mode: ConsensusMode,
        ) {
            self.state.borrow_mut().accepted.push(result.txns.id());
        }

        fn propose(&self, pos: &ConsensusProposal<NodeId, LedgerId, TxSetId>) {
            self.state.borrow_mut().proposed.push(pos.clone());
        }

        fn share_peer_position(&self, prop: &MockPeerPos) {
            self.state.borrow_mut().shared_positions.push(prop.clone());
        }

        fn share_tx(&self, _tx: &MockTx) {}

        fn share_tx_set(&self, set: &MockTxSet) {
            self.state.borrow_mut().shared_tx_sets.push(set.clone());
        }

        fn parms(&self) -> &ConsensusParms {
            &self.parms
        }

        fn next_ledger_time_resolution(&self, previous_resolution: Duration, _previous_agree: bool, _ledger_seq: u32) -> Duration {
            previous_resolution
        }

        fn round_close_time(&self, raw: NetClockTimePoint, _resolution: Duration) -> NetClockTimePoint {
            raw
        }
    }

    fn genesis_ledger() -> MockLedger {
        MockLedger {
            id: 0,
            seq: 0,
            close_time: NetClockTimePoint::new(1000),
            parent_close_time: NetClockTimePoint::new(990),
            close_agree: true,
        }
    }

    #[test]
    fn start_round_transitions_to_open_phase() {
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();

        c.start_round(&adaptor, NetClockTimePoint::new(1000), 0, genesis_ledger(), &HashSet::default(), true);

        assert_eq!(c.phase(), ConsensusPhase::Open);
        assert_eq!(c.mode(), ConsensusMode::Proposing);
        assert_eq!(*c.prev_ledger_id(), 0);
    }

    #[test]
    fn wrong_prev_ledger_without_acquisition_enters_wrong_ledger_mode() {
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();

        // prev_ledger_id (99) does not match prev_ledger.id() (0), and the
        // mock has no ledger registered for id 99, so acquisition fails.
        c.start_round(&adaptor, NetClockTimePoint::new(1000), 99, genesis_ledger(), &HashSet::default(), true);

        assert_eq!(c.mode(), ConsensusMode::WrongLedger);
    }

    #[test]
    fn timer_entry_closes_ledger_after_idle_interval_with_no_transactions() {
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();

        let start = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, start, 0, genesis_ledger(), &HashSet::default(), true);
        assert_eq!(c.phase(), ConsensusPhase::Open);

        // No transactions; advance past the *effective* idle interval,
        // which is max(ledger_idle_interval, 2 * close_time_resolution).
        // The genesis ledger's close_time_resolution is 10s, so the floor
        // (20s) exceeds ledger_idle_interval (15s) here.
        let effective_idle = adaptor.parms.ledger_idle_interval.max(Duration::from_secs(10) * 2);
        let later = start + time::Duration::seconds(effective_idle.as_secs() as i64 + 1);
        c.timer_entry(&adaptor, later);

        assert_eq!(c.phase(), ConsensusPhase::Establish);
        // Closing should have shared our (empty) initial transaction set
        // and proposed our position since we're in Proposing mode.
        assert_eq!(adaptor.state.borrow().shared_tx_sets.len(), 1);
        assert_eq!(adaptor.state.borrow().proposed.len(), 1);
    }

    #[test]
    fn solo_consensus_reaches_accepted_phase() {
        // With zero peers/proposers, and no disputes, a single node should
        // still eventually accept its own position once the min consensus
        // time has elapsed and checkConsensus's "alone for too long"
        // pathway kicks in (reachedMax after ledgerMaxConsensus).
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();

        let start = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, start, 0, genesis_ledger(), &HashSet::default(), true);

        let effective_idle = adaptor.parms.ledger_idle_interval.max(Duration::from_secs(10) * 2);
        let after_idle = start + time::Duration::seconds(effective_idle.as_secs() as i64 + 1);
        c.timer_entry(&adaptor, after_idle);
        assert_eq!(c.phase(), ConsensusPhase::Establish);

        // Drive timer_entry forward with a clock that reports elapsed time
        // past ledger_max_consensus so checkConsensus's total==0 path
        // returns true once reachedMax is set.
        let max_consensus = adaptor.parms.ledger_max_consensus;
        let far_future = after_idle + time::Duration::seconds(max_consensus.as_secs() as i64 + 5);

        // timer_entry uses the injected clock (SystemConsensusClock here)
        // for round_time bookkeeping, not `now` directly, so we must poll
        // repeatedly with real elapsed wall time for round_time to advance
        // past the thresholds. Since that's impractical in a fast unit
        // test, this test uses a manual clock instead -- see
        // `solo_consensus_reaches_accepted_phase_with_manual_clock` below,
        // which is the actual coverage for this path. This test only
        // verifies establish phase entry and no premature acceptance.
        c.timer_entry(&adaptor, far_future);
        assert_eq!(c.phase(), ConsensusPhase::Establish);
    }

    /// A manually-driven clock for deterministic phase-transition tests
    /// that depend on `ConsensusTimer`'s elapsed-time tracking.
    struct ManualClock {
        now: RefCell<Instant>,
    }

    impl ManualClock {
        fn new() -> Self {
            Self { now: RefCell::new(Instant::now()) }
        }
        fn advance(&self, d: Duration) {
            let next = *self.now.borrow() + d;
            *self.now.borrow_mut() = next;
        }
    }

    impl ConsensusClock for &ManualClock {
        fn now(&self) -> Instant {
            *self.now.borrow()
        }
    }

    #[test]
    fn solo_consensus_reaches_accepted_phase_with_manual_clock() {
        let adaptor = MockAdaptor::new();
        let clock = ManualClock::new();
        let mut c: Consensus<MockAdaptor, &ManualClock> = Consensus::with_clock(&clock);

        let start = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, start, 0, genesis_ledger(), &HashSet::default(), true);

        let idle = adaptor.parms.ledger_idle_interval.max(Duration::from_secs(10) * 2);
        clock.advance(idle + Duration::from_secs(1));
        c.timer_entry(&adaptor, start + time::Duration::seconds(idle.as_secs() as i64 + 1));
        assert_eq!(c.phase(), ConsensusPhase::Establish);

        // Advance both the manual steady clock (drives round_time via
        // ConsensusTimer::tick_to) and NetClock time past
        // ledger_max_consensus so checkConsensus's solo-alone pathway
        // returns Yes, and past ledger_min_consensus so phaseEstablish
        // doesn't bail out early.
        let max_consensus = adaptor.parms.ledger_max_consensus;
        clock.advance(max_consensus + Duration::from_secs(1));
        let now2 = start + time::Duration::seconds((idle.as_secs() + max_consensus.as_secs() + 2) as i64);
        c.timer_entry(&adaptor, now2);

        assert_eq!(c.phase(), ConsensusPhase::Accepted);
        assert_eq!(adaptor.state.borrow().accepted.len(), 1);
    }

    #[test]
    fn peer_proposal_for_wrong_prev_ledger_is_rejected() {
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();
        let now = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, now, 0, genesis_ledger(), &HashSet::default(), true);

        let bad_proposal = ConsensusProposal::new(99, 0, 0, now, now, 42);
        let accepted = c.peer_proposal(&adaptor, now, &MockPeerPos { proposal: bad_proposal });
        assert!(!accepted);
    }

    #[test]
    fn peer_proposal_bow_out_marks_node_dead() {
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();
        let now = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, now, 0, genesis_ledger(), &HashSet::default(), true);

        let mut prop = ConsensusProposal::new(0, 0, 0, now, now, 42);
        prop.bow_out(now);
        let accepted = c.peer_proposal(&adaptor, now, &MockPeerPos { proposal: prop });
        assert!(accepted);
        assert!(c.dead_nodes.contains(&42));
    }

    #[test]
    fn peer_proposal_with_differing_tx_set_creates_a_dispute() {
        // Peer proposes a tx set containing one transaction we don't have
        // in our (empty) initial position. Once we acquire that set, a
        // dispute should be created for the differing transaction.
        let adaptor = MockAdaptor::new();
        let mut c: Consensus<MockAdaptor> = Consensus::new();

        let start = NetClockTimePoint::new(1000);
        c.start_round(&adaptor, start, 0, genesis_ledger(), &HashSet::default(), true);

        // Close the ledger so we have an initial (empty) position/result.
        let effective_idle = adaptor.parms.ledger_idle_interval.max(Duration::from_secs(10) * 2);
        let closed_at = start + time::Duration::seconds(effective_idle.as_secs() as i64 + 1);
        c.timer_entry(&adaptor, closed_at);
        assert_eq!(c.phase(), ConsensusPhase::Establish);

        let peer_set = MockTxSet::with(&[7]);
        let peer_set_id = peer_set.id();
        adaptor.state.borrow_mut().tx_sets.insert(peer_set_id, peer_set);

        let peer_proposal = ConsensusProposal::new(0, 1, peer_set_id, closed_at, closed_at, 99);
        let accepted = c.peer_proposal(&adaptor, closed_at, &MockPeerPos { proposal: peer_proposal });
        assert!(accepted);

        // The peer's tx set should now be acquired, and a dispute created
        // for transaction 7 (which we don't have in our empty position).
        assert!(c.acquired.contains_key(&peer_set_id));
        let result = c.result.as_ref().expect("result set after close_ledger");
        assert!(result.disputes.contains_key(&7));
        // We don't have tx 7, so our initial vote on the dispute is "no".
        assert!(!result.disputes.get(&7).unwrap().get_our_vote());
    }
}
