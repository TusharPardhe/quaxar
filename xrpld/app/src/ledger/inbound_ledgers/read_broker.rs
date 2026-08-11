//! Bounded, unique-hash NodeStore reads for inbound state-tree acquisition.
//!
//! This module deliberately owns only read admission, coalescing, and result
//! settlement. It has no SHAMap, peer, registry, or acquisition mutex. The
//! actor submits [`ReadDispatch`] after releasing its own mutable state and
//! receives [`ReadReady`] through a mailbox sink after the broker settles.

use basics::base_uint::Uint256;
use nodestore::NodeObject;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// Maximum physical NodeStore reads owned by all inbound acquisitions in the
/// isolated high-throughput experiment.
pub const ACQ_READS_GLOBAL: usize = 128;
/// Maximum dispatched read subscriptions attributable to one acquisition in
/// the isolated high-throughput experiment.
pub const ACQ_READS_PER_ACQUISITION: usize = 32;
/// Maximum acquisitions/plans that may wait on one unique key.
pub const ACQ_READ_WAITERS_PER_KEY: usize = 32;

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
    WaitersPerKeyLimit,
    PerAcquisitionLimit,
}

/// Result of a submission attempt. Deferred requests retain their ticket and
/// wait in the broker FIFO; the actor can cancel that ticket before admission.
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
    pub per_acquisition_in_flight: usize,
    pub waiters_per_key: usize,
}

impl Default for ReadBrokerConfig {
    fn default() -> Self {
        Self {
            global_in_flight: ACQ_READS_GLOBAL,
            per_acquisition_in_flight: ACQ_READS_PER_ACQUISITION,
            waiters_per_key: ACQ_READ_WAITERS_PER_KEY,
        }
    }
}

impl ReadBrokerConfig {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.global_in_flight == 0 {
            return Err("global inbound read capacity must be nonzero");
        }
        if self.per_acquisition_in_flight == 0 {
            return Err("per-acquisition inbound read capacity must be nonzero");
        }
        if self.waiters_per_key == 0 {
            return Err("per-key inbound read waiter capacity must be nonzero");
        }
        Ok(self)
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
    pub stale_completions: u64,
    pub queue_high_water: usize,
    pub in_flight_high_water: usize,
    pub waiter_high_water: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadBrokerSnapshot {
    pub stopped: bool,
    pub queued_keys: usize,
    pub in_flight_keys: usize,
    pub active_by_acquisition: BTreeMap<u64, usize>,
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
    active_by_acquisition: BTreeMap<u64, usize>,
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
            active_by_acquisition: BTreeMap::new(),
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

        let dispatched = state
            .flights
            .get(&key)
            .is_some_and(|flight| flight.state == Some(FlightState::Dispatched));
        if dispatched
            && state
                .active_by_acquisition
                .get(&acquisition_id)
                .copied()
                .unwrap_or_default()
                >= self.inner.config.per_acquisition_in_flight
        {
            state.metrics.rejected += 1;
            return ReadAdmission::Rejected(ReadRejectReason::PerAcquisitionLimit);
        }
        if state
            .flights
            .get(&key)
            .is_some_and(|flight| flight.subscribers.len() >= self.inner.config.waiters_per_key)
        {
            state.metrics.rejected += 1;
            return ReadAdmission::Rejected(ReadRejectReason::WaitersPerKeyLimit);
        }

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
        if is_new_flight {
            state.fifo.push_back(key);
            let queued_keys = state.fifo.len();
            state.metrics.queue_high_water = state.metrics.queue_high_water.max(queued_keys);
        } else if dispatched {
            *state
                .active_by_acquisition
                .entry(acquisition_id)
                .or_default() += 1;
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
            if record.dispatched {
                Self::release_acquisition_slot(&mut state, subscriber.acquisition_id);
            }
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
                if record.dispatched {
                    Self::release_acquisition_slot(&mut state, subscriber.acquisition_id);
                }
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
            state.active_by_acquisition.clear();
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
            let callback = Box::new(move |object| completion.complete_from_node_store(object));
            match store {
                SHAMapStoreNodeStore::Single(database) => {
                    database.async_fetch(key.hash, key.ledger_seq, callback)
                }
                SHAMapStoreNodeStore::Rotating(database) => {
                    database.async_fetch(key.hash, key.ledger_seq, callback)
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
            in_flight_keys: state
                .flights
                .values()
                .filter(|flight| flight.state == Some(FlightState::Dispatched))
                .count(),
            active_by_acquisition: state.active_by_acquisition.clone(),
            metrics: state.metrics,
        }
    }

    fn admit_queued_locked(inner: &Arc<NodeReadBrokerInner>, state: &mut BrokerState) {
        let mut skipped = 0usize;
        while Self::in_flight_count(state) < inner.config.global_in_flight
            && !state.fifo.is_empty()
            && skipped < state.fifo.len()
        {
            let key = state.fifo.pop_front().expect("nonempty broker FIFO");
            let Some(flight) = state.flights.get(&key) else {
                continue;
            };
            if flight.state != Some(FlightState::Queued) || flight.subscribers.is_empty() {
                continue;
            }
            let subscriber_ids = flight.subscribers.keys().copied().collect::<Vec<_>>();
            let eligible = subscriber_ids.iter().all(|ticket_id| {
                let acquisition_id = state
                    .tickets
                    .get(ticket_id)
                    .expect("broker flight subscriber must have a ticket")
                    .acquisition_id;
                state
                    .active_by_acquisition
                    .get(&acquisition_id)
                    .copied()
                    .unwrap_or_default()
                    < inner.config.per_acquisition_in_flight
            });
            if !eligible {
                state.fifo.push_back(key);
                skipped += 1;
                continue;
            }
            skipped = 0;
            state
                .flights
                .get_mut(&key)
                .expect("queued broker flight must exist")
                .state = Some(FlightState::Dispatched);
            for ticket_id in subscriber_ids {
                let record = state
                    .tickets
                    .get_mut(&ticket_id)
                    .expect("broker ticket must exist for dispatched flight");
                record.dispatched = true;
                *state
                    .active_by_acquisition
                    .entry(record.acquisition_id)
                    .or_default() += 1;
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

    fn release_acquisition_slot(state: &mut BrokerState, acquisition_id: u64) {
        let remove = {
            let Some(active) = state.active_by_acquisition.get_mut(&acquisition_id) else {
                return;
            };
            *active = active.saturating_sub(1);
            *active == 0
        };
        if remove {
            state.active_by_acquisition.remove(&acquisition_id);
        }
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
    fn complete_from_node_store(mut self, object: Option<Arc<NodeObject>>) {
        self.settled = true;
        self.broker.complete_from_node_store(self.key, object);
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
            per_acquisition_in_flight: 1,
            waiters_per_key: 4,
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
            per_acquisition_in_flight: 1,
            waiters_per_key: 4,
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
    fn waiter_limit_rejects_overload_without_hiding_the_existing_flight() {
        let broker = broker(ReadBrokerConfig {
            global_in_flight: 1,
            per_acquisition_in_flight: 4,
            waiters_per_key: 1,
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            broker.request(key(10, 10), 1, 1, sink(Arc::clone(&events))),
            ReadAdmission::Accepted(_)
        ));
        assert_eq!(
            broker.request(key(10, 10), 2, 1, sink(Arc::clone(&events))),
            ReadAdmission::Rejected(ReadRejectReason::WaitersPerKeyLimit)
        );
        let dispatch = broker
            .take_ready_dispatches()
            .into_iter()
            .next()
            .expect("dispatch");
        dispatch.complete(ReadOutcome::Miss);
        assert_eq!(events.lock().expect("event sink").len(), 1);
        assert_eq!(broker.snapshot().metrics.rejected, 1);
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
