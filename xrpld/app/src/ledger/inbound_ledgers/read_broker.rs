//! Bounded, unique-hash NodeStore reads for inbound state-tree acquisition.
//!
//! This module deliberately owns only read admission, coalescing, and result
//! settlement. It has no SHAMap, peer, registry, or acquisition mutex. The
//! actor submits [`ReadDispatch`] after releasing its own mutable state and
//! receives [`ReadReady`] through a mailbox sink after the broker settles.

use basics::base_uint::Uint256;
use nodestore::{AsyncReadWork, NodeObject};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// The sole explicit NodeStore resource boundary for inbound traversal.
///
/// Rippled retains up to 512 deferred reads in one `getMissingNodes()` pass.
/// Quaxar preserves that progression while making the physical-read limit
/// global, so coalesced acquisitions never multiply database I/O.
pub const ACQ_READS_GLOBAL: usize = 512;

/// Bounded retained logical ownership for the global Rust broker.
///
/// Rippled's 512 limit is call-local to one `getMissingNodes()` pass; it does
/// not define a global broker queue, a 513th successor, or a cancellation
/// event on admission pressure. Quaxar has a shared physical-read broker, so
/// its one retained logical-subscription budget is the already-configured
/// physical limit itself. One subscription owns one ticket and one waiter, so
/// it bounds retained tickets, waiters, and queued keys without inventing an
/// independent successor limit. A cancelled dispatched callback may outlive
/// its subscriber, but remains within the existing physical in-flight limit.
/// A request at the retained-subscription bound is returned as non-terminal
/// `Deferred` with no broker record or sink event, leaving the actor's
/// retained missing edge available for bounded local-read retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadBrokerLimits {
    pub max_retained_logical_subscriptions: usize,
}

impl ReadBrokerLimits {
    const fn from_physical_limit(global_in_flight: usize) -> Self {
        Self {
            max_retained_logical_subscriptions: global_in_flight,
        }
    }
}

/// One database identity. The generation makes a rotation boundary explicit:
/// equal hashes from different backing generations never share a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReadKey {
    pub hash: Uint256,
    pub ledger_seq: u32,
    pub database_generation: u64,
}

impl ReadKey {
    pub const fn new(hash: Uint256, ledger_seq: u32, database_generation: u64) -> Self {
        Self {
            hash,
            ledger_seq,
            database_generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReadTicketId(u64);

impl ReadTicketId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Cancellation handle for a logical acquisition subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadTicket {
    id: ReadTicketId,
    key: ReadKey,
    acquisition_id: u64,
    plan_id: u64,
}

impl ReadTicket {
    pub const fn id(self) -> ReadTicketId {
        self.id
    }

    pub const fn key(self) -> ReadKey {
        self.key
    }

    pub const fn acquisition_id(self) -> u64 {
        self.acquisition_id
    }

    pub const fn plan_id(self) -> u64 {
        self.plan_id
    }
}

/// Explicit terminal outcome for every logical broker subscription.
#[derive(Debug, Clone)]
pub enum ReadOutcome {
    Found(Arc<NodeObject>),
    Miss,
    Cancelled,
    Fault(Arc<str>),
}

impl PartialEq for ReadOutcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Found(left), Self::Found(right)) => {
                left.hash() == right.hash() && left.data() == right.data()
            }
            (Self::Miss, Self::Miss) | (Self::Cancelled, Self::Cancelled) => true,
            (Self::Fault(left), Self::Fault(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for ReadOutcome {}

/// One actor-mailbox event. The sink receives at most one event per ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReady {
    pub ticket: ReadTicket,
    pub outcome: ReadOutcome,
}

pub type ReadReadySink = Arc<dyn Fn(ReadReady) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRejectReason {
    Stopped,
}

/// Result of a submission attempt. `Deferred` can retain a broker ticket
/// while physical dispatch is occupied. At the retained-subscription bound it
/// has no broker record and produces no terminal sink event: that admission
/// signal leaves the caller's missing edge available for bounded local-read retry.
/// Cancelling an unretained ticket is harmless and returns `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAdmission {
    Accepted(ReadTicket),
    Attached(ReadTicket),
    Deferred(ReadTicket),
    Rejected(ReadRejectReason),
}

/// All broker bounds are centralized here so actor and registry code never
/// smuggle capacities in as unrelated literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadBrokerConfig {
    pub global_in_flight: usize,
}

impl Default for ReadBrokerConfig {
    fn default() -> Self {
        Self {
            global_in_flight: ACQ_READS_GLOBAL,
        }
    }
}

impl ReadBrokerConfig {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.global_in_flight == 0 {
            return Err("global inbound read capacity must be nonzero");
        }
        Ok(self)
    }

    /// Stable Worker 3 handoff for Worker 2: admission observes this finite
    /// budget before creating a ticket, and the `ReadKey` generation supplied
    /// by Worker 2 must be the store generation observed at that same point.
    pub const fn logical_limits(self) -> ReadBrokerLimits {
        ReadBrokerLimits::from_physical_limit(self.global_in_flight)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadBrokerMetrics {
    pub submitted: u64,
    pub attached: u64,
    pub deferred: u64,
    pub rejected: u64,
    pub physical_dispatched: u64,
    pub found: u64,
    pub misses: u64,
    pub faults: u64,
    pub cancelled: u64,
    /// Capacity was exhausted before another broker-owned subscription was
    /// retained. The request is returned as non-terminal `Deferred`; no
    /// `ReadOutcome::Cancelled` actor event is fabricated.
    pub capacity_deferred: u64,
    pub stale_completions: u64,
    pub queue_high_water: usize,
    pub in_flight_high_water: usize,
    pub waiter_high_water: usize,
    pub logical_ticket_high_water: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadBrokerSnapshot {
    pub stopped: bool,
    pub queued_keys: usize,
    pub queued_key_bytes: usize,
    pub in_flight_keys: usize,
    pub logical_tickets: usize,
    pub logical_ticket_bytes: usize,
    pub waiters: usize,
    pub waiter_bytes: usize,
    pub limits: ReadBrokerLimits,
    pub metrics: ReadBrokerMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlightState {
    Queued,
    Dispatched,
}

#[derive(Clone)]
struct Subscriber {
    acquisition_id: u64,
    plan_id: u64,
    sink: ReadReadySink,
}

#[derive(Default)]
struct Flight {
    state: Option<FlightState>,
    subscribers: BTreeMap<ReadTicketId, Subscriber>,
    by_owner: BTreeMap<(u64, u64), ReadTicketId>,
}

struct TicketRecord {
    key: ReadKey,
    acquisition_id: u64,
    plan_id: u64,
    dispatched: bool,
}

struct BrokerState {
    stopped: bool,
    next_ticket: u64,
    flights: BTreeMap<ReadKey, Flight>,
    fifo: VecDeque<ReadKey>,
    ready_dispatches: VecDeque<ReadDispatch>,
    tickets: BTreeMap<ReadTicketId, TicketRecord>,
    metrics: ReadBrokerMetrics,
}

impl Default for BrokerState {
    fn default() -> Self {
        Self {
            stopped: false,
            next_ticket: 1,
            flights: BTreeMap::new(),
            fifo: VecDeque::new(),
            ready_dispatches: VecDeque::new(),
            tickets: BTreeMap::new(),
            metrics: ReadBrokerMetrics::default(),
        }
    }
}

struct NodeReadBrokerInner {
    config: ReadBrokerConfig,
    state: Mutex<BrokerState>,
}

/// Shared broker owner. Clone this into registry-independent acquisition
/// handles; no clone grants access to an acquisition's mutable planner.
#[derive(Clone)]
pub struct NodeReadBroker {
    inner: Arc<NodeReadBrokerInner>,
}

impl NodeReadBroker {
    pub fn new(config: ReadBrokerConfig) -> Result<Self, &'static str> {
        Ok(Self {
            inner: Arc::new(NodeReadBrokerInner {
                config: config.validate()?,
                state: Mutex::new(BrokerState::default()),
            }),
        })
    }

    pub fn request(
        &self,
        key: ReadKey,
        acquisition_id: u64,
        plan_id: u64,
        sink: ReadReadySink,
    ) -> ReadAdmission {
        let mut state = self.inner.state.lock().expect("read broker state lock");
        if state.stopped {
            state.metrics.rejected += 1;
            return ReadAdmission::Rejected(ReadRejectReason::Stopped);
        }

        let owner = (acquisition_id, plan_id);
        let existing_ticket = state.flights.get(&key).and_then(|flight| {
            flight.by_owner.get(&owner).and_then(|ticket_id| {
                state
                    .tickets
                    .get(ticket_id)
                    .map(|record| (*ticket_id, record.acquisition_id, record.plan_id))
            })
        });
        if let Some((ticket_id, ticket_acquisition_id, ticket_plan_id)) = existing_ticket {
            state.metrics.attached += 1;
            return ReadAdmission::Attached(ReadTicket {
                id: ticket_id,
                key,
                acquisition_id: ticket_acquisition_id,
                plan_id: ticket_plan_id,
            });
        }

        let limits = self.inner.config.logical_limits();
        if state.tickets.len() >= limits.max_retained_logical_subscriptions {
            let ticket_id = ReadTicketId(state.next_ticket);
            state.next_ticket = state
                .next_ticket
                .checked_add(1)
                .expect("read ticket id overflow");
            let ticket = ReadTicket {
                id: ticket_id,
                key,
                acquisition_id,
                plan_id,
            };
            state.metrics.submitted += 1;
            state.metrics.deferred += 1;
            state.metrics.capacity_deferred += 1;
            return ReadAdmission::Deferred(ticket);
        }

        let dispatched = state
            .flights
            .get(&key)
            .is_some_and(|flight| flight.state == Some(FlightState::Dispatched));

        let ticket_id = ReadTicketId(state.next_ticket);
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .expect("read ticket id overflow");
        let ticket = ReadTicket {
            id: ticket_id,
            key,
            acquisition_id,
            plan_id,
        };
        state.metrics.submitted += 1;
        state.tickets.insert(
            ticket_id,
            TicketRecord {
                key,
                acquisition_id,
                plan_id,
                dispatched,
            },
        );
        let is_new_flight = !state.flights.contains_key(&key);
        let waiter_count = {
            let flight = state.flights.entry(key).or_default();
            flight.subscribers.insert(
                ticket_id,
                Subscriber {
                    acquisition_id,
                    plan_id,
                    sink,
                },
            );
            flight.by_owner.insert(owner, ticket_id);
            if is_new_flight {
                flight.state = Some(FlightState::Queued);
            }
            flight.subscribers.len()
        };
        state.metrics.waiter_high_water = state.metrics.waiter_high_water.max(waiter_count);
        state.metrics.logical_ticket_high_water = state
            .metrics
            .logical_ticket_high_water
            .max(state.tickets.len());
        if is_new_flight {
            state.fifo.push_back(key);
            let queued_keys = state.fifo.len();
            state.metrics.queue_high_water = state.metrics.queue_high_water.max(queued_keys);
        }

        Self::admit_queued_locked(&self.inner, &mut state);
        let admitted = state
            .tickets
            .get(&ticket_id)
            .is_some_and(|record| record.dispatched);
        if admitted {
            ReadAdmission::Accepted(ticket)
        } else {
            state.metrics.deferred += 1;
            ReadAdmission::Deferred(ticket)
        }
    }

    /// Cancel one logical subscription. A late physical completion is consumed
    /// but can never revive this ticket or deliver a second event.
    pub fn cancel(&self, ticket: ReadTicket) -> bool {
        let notification = {
            let mut state = self.inner.state.lock().expect("read broker state lock");
            let Some(existing) = state.tickets.get(&ticket.id) else {
                return false;
            };
            if existing.key != ticket.key
                || existing.acquisition_id != ticket.acquisition_id
                || existing.plan_id != ticket.plan_id
            {
                return false;
            }
            let record = state
                .tickets
                .remove(&ticket.id)
                .expect("validated broker ticket must still exist");
            let (subscriber, flight_state, empty) = {
                let Some(flight) = state.flights.get_mut(&record.key) else {
                    return false;
                };
                let Some(subscriber) = flight.subscribers.remove(&ticket.id) else {
                    return false;
                };
                flight
                    .by_owner
                    .remove(&(subscriber.acquisition_id, subscriber.plan_id));
                (subscriber, flight.state, flight.subscribers.is_empty())
            };
            if empty && flight_state == Some(FlightState::Queued) {
                state.flights.remove(&record.key);
                state.fifo.retain(|queued| *queued != record.key);
            }
            state.metrics.cancelled += 1;
            Self::admit_queued_locked(&self.inner, &mut state);
            Some((
                subscriber.sink,
                ReadReady {
                    ticket,
                    outcome: ReadOutcome::Cancelled,
                },
            ))
        };
        if let Some((sink, ready)) = notification {
            sink(ready);
        }
        true
    }

    /// Settle a physical read. It is safe for a NodeStore callback to call this
    /// directly: actor mutation is delegated to the mailbox sinks after the
    /// broker mutex is released.
    pub fn complete(&self, key: ReadKey, outcome: ReadOutcome) -> bool {
        let notifications = {
            let mut state = self.inner.state.lock().expect("read broker state lock");
            let Some(flight) = state.flights.remove(&key) else {
                state.metrics.stale_completions += 1;
                return false;
            };
            if flight.state != Some(FlightState::Dispatched) {
                state.metrics.stale_completions += 1;
                return false;
            }
            let outcome = match outcome {
                ReadOutcome::Found(object) if object.hash() != &key.hash => {
                    ReadOutcome::Fault(Arc::from("NodeStore returned a different hash"))
                }
                outcome => outcome,
            };
            match &outcome {
                ReadOutcome::Found(_) => state.metrics.found += 1,
                ReadOutcome::Miss => state.metrics.misses += 1,
                ReadOutcome::Cancelled => state.metrics.cancelled += 1,
                ReadOutcome::Fault(_) => state.metrics.faults += 1,
            }
            let mut notifications = Vec::with_capacity(flight.subscribers.len());
            for (ticket_id, subscriber) in flight.subscribers {
                let Some(record) = state.tickets.remove(&ticket_id) else {
                    continue;
                };
                notifications.push((
                    subscriber.sink,
                    ReadReady {
                        ticket: ReadTicket {
                            id: ticket_id,
                            key,
                            acquisition_id: subscriber.acquisition_id,
                            plan_id: subscriber.plan_id,
                        },
                        outcome: outcome.clone(),
                    },
                ));
            }
            Self::admit_queued_locked(&self.inner, &mut state);
            notifications
        };
        for (sink, ready) in notifications {
            sink(ready);
        }
        true
    }

    pub fn complete_from_node_store(&self, key: ReadKey, object: Option<Arc<NodeObject>>) -> bool {
        self.complete(
            key,
            object.map(ReadOutcome::Found).unwrap_or(ReadOutcome::Miss),
        )
    }

    /// Explicit stop settlement. Queued and dispatched tickets are all
    /// cancelled; callback drops after NodeStore stop then become stale rather
    /// than silently stranding an acquisition.
    pub fn stop(&self) {
        let notifications = {
            let mut state = self.inner.state.lock().expect("read broker state lock");
            if state.stopped {
                return;
            }
            state.stopped = true;
            let flights = std::mem::take(&mut state.flights);
            state.fifo.clear();
            for mut dispatch in state.ready_dispatches.drain(..) {
                // Stop settles every ticket below. Suppress the dispatch Drop
                // fallback because it would otherwise re-enter this mutex.
                dispatch.settled = true;
            }
            let mut notifications = Vec::new();
            for (key, flight) in flights {
                for (ticket_id, subscriber) in flight.subscribers {
                    state.tickets.remove(&ticket_id);
                    state.metrics.cancelled += 1;
                    notifications.push((
                        subscriber.sink,
                        ReadReady {
                            ticket: ReadTicket {
                                id: ticket_id,
                                key,
                                acquisition_id: subscriber.acquisition_id,
                                plan_id: subscriber.plan_id,
                            },
                            outcome: ReadOutcome::Cancelled,
                        },
                    ));
                }
            }
            notifications
        };
        for (sink, ready) in notifications {
            sink(ready);
        }
    }

    /// Commands are removed from the broker under its lock but run by the
    /// caller afterwards. This is the ownership boundary that prevents an
    /// actor lock from spanning NodeStore submission.
    pub fn take_ready_dispatches(&self) -> Vec<ReadDispatch> {
        let mut state = self.inner.state.lock().expect("read broker state lock");
        state.ready_dispatches.drain(..).collect()
    }

    /// Submit all already-admitted physical reads. The callback captures only
    /// a settlement handle; it never accesses acquisition or registry state.
    pub fn submit_ready_to_node_store(&self, store: &SHAMapStoreNodeStore) -> usize {
        let dispatches = self.take_ready_dispatches();
        let count = dispatches.len();
        for dispatch in dispatches {
            let key = dispatch.key();
            let completion = dispatch.into_completion();
            let work: Box<dyn AsyncReadWork> = Box::new(completion);
            match store {
                SHAMapStoreNodeStore::Single(database) => {
                    database.async_fetch(key.hash, key.ledger_seq, work)
                }
                SHAMapStoreNodeStore::Rotating(database) => {
                    database.async_fetch(key.hash, key.ledger_seq, work)
                }
            }
        }
        count
    }

    pub fn snapshot(&self) -> ReadBrokerSnapshot {
        let state = self.inner.state.lock().expect("read broker state lock");
        ReadBrokerSnapshot {
            stopped: state.stopped,
            queued_keys: state.fifo.len(),
            queued_key_bytes: state
                .fifo
                .len()
                .saturating_mul(std::mem::size_of::<ReadKey>()),
            in_flight_keys: state
                .flights
                .values()
                .filter(|flight| flight.state == Some(FlightState::Dispatched))
                .count(),
            logical_tickets: state.tickets.len(),
            logical_ticket_bytes: state
                .tickets
                .len()
                .saturating_mul(std::mem::size_of::<TicketRecord>()),
            waiters: state
                .flights
                .values()
                .map(|flight| flight.subscribers.len())
                .sum(),
            waiter_bytes: state
                .flights
                .values()
                .map(|flight| flight.subscribers.len())
                .sum::<usize>()
                .saturating_mul(std::mem::size_of::<Subscriber>()),
            limits: self.inner.config.logical_limits(),
            metrics: state.metrics,
        }
    }

    fn admit_queued_locked(inner: &Arc<NodeReadBrokerInner>, state: &mut BrokerState) {
        while Self::in_flight_count(state) < inner.config.global_in_flight {
            let Some(key) = state.fifo.pop_front() else {
                break;
            };
            let Some(flight) = state.flights.get(&key) else {
                continue;
            };
            if flight.state != Some(FlightState::Queued) || flight.subscribers.is_empty() {
                continue;
            }
            let subscriber_ids = flight.subscribers.keys().copied().collect::<Vec<_>>();
            state
                .flights
                .get_mut(&key)
                .expect("queued broker flight must exist")
                .state = Some(FlightState::Dispatched);
            for ticket_id in subscriber_ids {
                state
                    .tickets
                    .get_mut(&ticket_id)
                    .expect("broker ticket must exist for dispatched flight")
                    .dispatched = true;
            }
            state.ready_dispatches.push_back(ReadDispatch {
                broker: NodeReadBroker {
                    inner: Arc::clone(inner),
                },
                key,
                settled: false,
            });
            state.metrics.physical_dispatched += 1;
            let in_flight = Self::in_flight_count(state);
            state.metrics.in_flight_high_water = state.metrics.in_flight_high_water.max(in_flight);
        }
    }

    fn in_flight_count(state: &BrokerState) -> usize {
        state
            .flights
            .values()
            .filter(|flight| flight.state == Some(FlightState::Dispatched))
            .count()
    }
}

/// One admitted physical NodeStore read. Dropping it settles the associated
/// flight as cancelled, ensuring failed submission cannot leak broker capacity.
pub struct ReadDispatch {
    broker: NodeReadBroker,
    key: ReadKey,
    settled: bool,
}

impl ReadDispatch {
    pub const fn key(&self) -> ReadKey {
        self.key
    }

    pub fn complete(mut self, outcome: ReadOutcome) {
        self.settled = true;
        self.broker.complete(self.key, outcome);
    }

    fn into_completion(mut self) -> ReadCompletion {
        self.settled = true;
        ReadCompletion {
            broker: self.broker.clone(),
            key: self.key,
            settled: false,
        }
    }
}

impl Drop for ReadDispatch {
    fn drop(&mut self) {
        if !self.settled {
            self.broker.complete(self.key, ReadOutcome::Cancelled);
        }
    }
}

/// Callback-only settlement handle. If NodeStore stops by dropping its queued
/// callback, this handle's Drop implementation converts that loss into an
/// explicit cancelled outcome.
struct ReadCompletion {
    broker: NodeReadBroker,
    key: ReadKey,
    settled: bool,
}

impl ReadCompletion {
    fn complete_from_node_store(&mut self, object: Option<Arc<NodeObject>>) {
        self.settled = true;
        self.broker.complete_from_node_store(self.key, object);
    }
}

impl AsyncReadWork for ReadCompletion {
    fn complete(&mut self, object: Option<Arc<NodeObject>>) {
        self.complete_from_node_store(object);
    }
}

impl Drop for ReadCompletion {
    fn drop(&mut self) {
        if !self.settled {
            self.broker.complete(self.key, ReadOutcome::Cancelled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn key(byte: u8, seq: u32) -> ReadKey {
        ReadKey::new(Uint256::from_array([byte; 32]), seq, 1)
    }

    fn sink(events: Arc<Mutex<Vec<ReadReady>>>) -> ReadReadySink {
        Arc::new(move |ready| events.lock().expect("event sink lock").push(ready))
    }

    fn broker(config: ReadBrokerConfig) -> NodeReadBroker {
        NodeReadBroker::new(config).expect("valid broker config")
    }

    #[test]
    fn duplicate_plan_hash_has_one_ticket_one_dispatch_and_one_result() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let first = broker.request(key(1, 10), 7, 3, sink(Arc::clone(&events)));
        let ticket = match first {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected immediate admission, got {other:?}"),
        };
        assert_eq!(
            broker.request(key(1, 10), 7, 3, sink(Arc::clone(&events))),
            ReadAdmission::Attached(ticket)
        );
        let dispatches = broker.take_ready_dispatches();
        assert_eq!(dispatches.len(), 1);
        dispatches
            .into_iter()
            .next()
            .expect("dispatch")
            .complete(ReadOutcome::Miss);
        let events = events.lock().expect("event sink lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ticket, ticket);
        assert_eq!(events[0].outcome, ReadOutcome::Miss);
    }

    #[test]
    fn shared_key_fans_out_once_per_acquisition_without_duplicate_io() {
        let broker = broker(ReadBrokerConfig::default());
        let first_events = Arc::new(Mutex::new(Vec::new()));
        let second_events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(2, 10), 1, 11, sink(Arc::clone(&first_events))),
            ReadAdmission::Accepted(_)
        ));
        assert!(matches!(
            broker.request(key(2, 10), 2, 12, sink(Arc::clone(&second_events))),
            ReadAdmission::Accepted(_)
        ));
        let dispatches = broker.take_ready_dispatches();
        assert_eq!(dispatches.len(), 1);
        dispatches
            .into_iter()
            .next()
            .expect("dispatch")
            .complete(ReadOutcome::Miss);
        assert_eq!(first_events.lock().expect("first sink").len(), 1);
        assert_eq!(second_events.lock().expect("second sink").len(), 1);
        assert_eq!(broker.snapshot().metrics.physical_dispatched, 1);
    }

    #[test]
    fn distinct_database_sequences_do_not_share_a_physical_read() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(3, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        assert!(matches!(
            broker.request(key(3, 11), 2, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        let dispatches = broker.take_ready_dispatches();
        assert_eq!(dispatches.len(), 2);
    }

    #[test]
    fn deferred_fifo_admission_recovers_after_incremental_completion() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let first = broker.request(key(4, 10), 1, 1, sink(Arc::clone(&events)));
        let second = broker.request(key(5, 10), 2, 1, sink(Arc::clone(&events)));
        assert!(matches!(first, ReadAdmission::Accepted(_)));
        assert!(matches!(second, ReadAdmission::Deferred(_)));
        let first_dispatch = broker.take_ready_dispatches();
        assert_eq!(first_dispatch.len(), 1);
        first_dispatch
            .into_iter()
            .next()
            .expect("first dispatch")
            .complete(ReadOutcome::Miss);
        let second_dispatch = broker.take_ready_dispatches();
        assert_eq!(second_dispatch.len(), 1);
        second_dispatch
            .into_iter()
            .next()
            .expect("second dispatch")
            .complete(ReadOutcome::Miss);
        assert_eq!(events.lock().expect("event sink").len(), 2);
        assert_eq!(broker.snapshot().in_flight_keys, 0);
    }

    #[test]
    fn cancellation_settles_once_and_late_completion_cannot_resurrect() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let ticket = match broker.request(key(6, 10), 1, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected accepted ticket, got {other:?}"),
        };
        let dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("dispatch");
        assert!(broker.cancel(ticket));
        dispatch.complete(ReadOutcome::Miss);
        let events = events.lock().expect("event sink");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, ReadOutcome::Cancelled);
        assert_eq!(broker.snapshot().metrics.stale_completions, 1);
    }

    #[test]
    fn stop_cancels_queued_and_dispatched_subscribers() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(7, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        assert!(matches!(
            broker.request(key(8, 10), 2, 1, sink(Arc::clone(&events))),
            ReadAdmission::Deferred(_)
        ));
        broker.stop();
        let outcomes = events
            .lock()
            .expect("event sink")
            .iter()
            .map(|event| event.outcome.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes,
            vec![ReadOutcome::Cancelled, ReadOutcome::Cancelled]
        );
        assert!(broker.snapshot().stopped);
    }

    #[test]
    fn shared_key_attaches_without_an_artificial_waiter_limit() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(10, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        assert!(matches!(
            broker.request(key(10, 10), 2, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        let dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("dispatch");
        dispatch.complete(ReadOutcome::Miss);
        assert_eq!(events.lock().expect("event sink").len(), 2);
        assert_eq!(broker.snapshot().metrics.rejected, 0);
    }

    #[test]
    fn request_at_real_512_513_514_boundary_defers_without_actor_completion() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut tickets = Vec::with_capacity(ACQ_READS_GLOBAL);
        for sequence in 0..ACQ_READS_GLOBAL as u32 {
            let admission = broker.request(
                key((sequence % u8::MAX as u32) as u8, sequence),
                u64::from(sequence) + 1,
                1,
                sink(Arc::clone(&events)),
            );
            let ReadAdmission::Accepted(ticket) = admission else {
                panic!("request {sequence} must fit the real 512-read boundary: {admission:?}");
            };
            tickets.push(ticket);
        }

        let deferred_513 = broker.request(
            key(0xFE, ACQ_READS_GLOBAL as u32),
            513,
            1,
            sink(Arc::clone(&events)),
        );
        let deferred_514 = broker.request(
            key(0xFD, ACQ_READS_GLOBAL as u32 + 1),
            514,
            1,
            sink(Arc::clone(&events)),
        );
        let ReadAdmission::Deferred(ticket_513) = deferred_513 else {
            panic!("513th distinct request must be capacity deferred");
        };
        let ReadAdmission::Deferred(ticket_514) = deferred_514 else {
            panic!("514th distinct request must be capacity deferred");
        };

        let snapshot = broker.snapshot();
        assert_eq!(snapshot.limits.max_retained_logical_subscriptions, 512);
        assert_eq!(snapshot.logical_tickets, 512);
        assert_eq!(snapshot.in_flight_keys, 512);
        assert_eq!(snapshot.queued_keys, 0);
        assert_eq!(snapshot.metrics.capacity_deferred, 2);
        assert_eq!(snapshot.metrics.cancelled, 0);
        assert!(events.lock().expect("event sink").is_empty());
        assert!(!broker.cancel(ticket_513));
        assert!(!broker.cancel(ticket_514));

        let dispatches = broker.take_ready_dispatches();
        assert_eq!(dispatches.len(), 512);
        for dispatch in dispatches {
            dispatch.complete(ReadOutcome::Miss);
        }
        let events = events.lock().expect("event sink");
        assert_eq!(events.len(), 512);
        assert!(
            events
                .iter()
                .all(|ready| ready.outcome == ReadOutcome::Miss)
        );
    }

    #[test]
    fn cancellation_is_the_only_cancelled_actor_event_and_reopens_capacity_after_settlement() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let ticket = match broker.request(key(12, 10), 1, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected accepted ticket, got {other:?}"),
        };
        let dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("dispatch");
        let deferred = match broker.request(key(13, 10), 2, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Deferred(ticket) => ticket,
            other => panic!("expected capacity defer, got {other:?}"),
        };
        assert!(!broker.cancel(deferred));
        assert!(broker.cancel(ticket));
        dispatch.complete(ReadOutcome::Miss);
        assert_eq!(
            events.lock().expect("event sink").as_slice(),
            &[ReadReady {
                ticket,
                outcome: ReadOutcome::Cancelled,
            }]
        );

        let replacement = match broker.request(key(14, 10), 3, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected post-settlement admission, got {other:?}"),
        };
        let replacement_dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("replacement dispatch");
        replacement_dispatch.complete(ReadOutcome::Miss);
        assert_eq!(
            events.lock().expect("event sink").last(),
            Some(&ReadReady {
                ticket: replacement,
                outcome: ReadOutcome::Miss,
            })
        );
    }

    #[test]
    fn pre_and_post_store_generation_settle_independently_without_capacity_cancellation() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let hash = Uint256::from_array([0xD4; 32]);
        let before_rotation = ReadKey::new(hash, 44, 7);
        let after_rotation = ReadKey::new(hash, 44, 8);
        let before_ticket = match broker.request(before_rotation, 1, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected pre-rotation admission, got {other:?}"),
        };
        let before_dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("pre-rotation dispatch");
        let deferred_after_rotation =
            match broker.request(after_rotation, 2, 1, sink(Arc::clone(&events))) {
                ReadAdmission::Deferred(ticket) => ticket,
                other => panic!("expected post-rotation capacity defer, got {other:?}"),
            };
        assert!(events.lock().expect("event sink").is_empty());
        assert!(!broker.cancel(deferred_after_rotation));

        before_dispatch.complete(ReadOutcome::Miss);
        let after_ticket = match broker.request(after_rotation, 2, 1, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected post-rotation retry admission, got {other:?}"),
        };
        let after_dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("post-rotation dispatch");
        after_dispatch.complete(ReadOutcome::Miss);

        assert_eq!(
            events.lock().expect("event sink").as_slice(),
            &[
                ReadReady {
                    ticket: before_ticket,
                    outcome: ReadOutcome::Miss,
                },
                ReadReady {
                    ticket: after_ticket,
                    outcome: ReadOutcome::Miss,
                },
            ]
        );
    }

    #[test]
    fn late_pre_rotation_completion_cannot_settle_the_replacement_generation_ticket() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 2,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let hash = Uint256::from_array([0xE1; 32]);
        let before = ReadKey::new(hash, 77, 41);
        let after = ReadKey::new(hash, 77, 42);
        let before_ticket = match broker.request(before, 1, 9, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected old-generation admission, got {other:?}"),
        };
        let after_ticket = match broker.request(after, 1, 10, sink(Arc::clone(&events))) {
            ReadAdmission::Accepted(ticket) => ticket,
            other => panic!("expected replacement-generation admission, got {other:?}"),
        };
        let mut dispatches = broker.take_ready_dispatches();
        assert_eq!(dispatches.len(), 2);
        let before_dispatch = dispatches
            .iter()
            .position(|dispatch| dispatch.key() == before)
            .map(|index| dispatches.remove(index))
            .expect("old-generation dispatch");
        let after_dispatch = dispatches
            .into_iter()
            .next()
            .expect("replacement-generation dispatch");

        // The old backend lifetime remains valid until this callback settles,
        // but its key cannot complete the replacement owner.
        before_dispatch.complete(ReadOutcome::Miss);
        assert_eq!(
            events.lock().expect("event sink").as_slice(),
            &[ReadReady {
                ticket: before_ticket,
                outcome: ReadOutcome::Miss,
            }]
        );
        assert_eq!(broker.snapshot().logical_tickets, 1);
        after_dispatch.complete(ReadOutcome::Miss);
        assert_eq!(
            events.lock().expect("event sink").as_slice(),
            &[
                ReadReady {
                    ticket: before_ticket,
                    outcome: ReadOutcome::Miss,
                },
                ReadReady {
                    ticket: after_ticket,
                    outcome: ReadOutcome::Miss,
                },
            ]
        );
        assert_eq!(broker.snapshot().logical_tickets, 0);
    }

    #[test]
    fn backend_fault_settles_the_ticket_once() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(11, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("dispatch")
            .complete(ReadOutcome::Fault(Arc::from("backend read failed")));
        let events = events.lock().expect("event sink");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].outcome,
            ReadOutcome::Fault(Arc::from("backend read failed"))
        );
        assert_eq!(broker.snapshot().metrics.faults, 1);
    }

    #[test]
    fn dropped_dispatch_settles_as_cancelled() {
        let broker = broker(ReadBrokerConfig::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(9, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        drop(broker.take_ready_dispatches());
        let events = events.lock().expect("event sink");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, ReadOutcome::Cancelled);
    }
}
