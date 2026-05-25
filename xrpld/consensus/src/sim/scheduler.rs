//! Discrete-event scheduler with timers.
//!
//!
//! Provides a priority queue of timed callbacks driven by a manual clock.
//! Events are processed in time order; the clock advances to each event's time.

use super::types::{SimDuration, SimTime};
use std::collections::BinaryHeap;

/// Unique token for cancelling a scheduled event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CancelToken(u64);

/// A scheduled event.
#[allow(dead_code)]
struct Event {
    when: SimTime,
    id: u64,
    handler: Box<dyn FnOnce()>,
    cancelled: bool,
}

/// Wrapper for BinaryHeap ordering (earliest first).
struct EventEntry {
    when: SimTime,
    id: u64,
}

impl PartialEq for EventEntry {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when && self.id == other.id
    }
}
impl Eq for EventEntry {}

impl PartialOrd for EventEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for EventEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse: smallest time first, then smallest id
        other.when.cmp(&self.when).then(other.id.cmp(&self.id))
    }
}

/// Simulated discrete-event scheduler.
///
/// Events are closures scheduled at specific times. The clock advances
/// to each event's time as it's processed.
pub struct Scheduler {
    now: SimTime,
    next_id: u64,
    queue: BinaryHeap<EventEntry>,
    events: Vec<Option<Event>>,
    cancelled: std::collections::HashSet<u64>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            now: SimTime::ZERO,
            next_id: 0,
            queue: BinaryHeap::new(),
            events: Vec::new(),
            cancelled: std::collections::HashSet::new(),
        }
    }

    /// Current simulation time.
    pub fn now(&self) -> SimTime {
        self.now
    }

    /// Schedule an event at a specific time.
    pub fn at(&mut self, when: SimTime, f: impl FnOnce() + 'static) -> CancelToken {
        let id = self.next_id;
        self.next_id += 1;

        self.queue.push(EventEntry { when, id });
        // Store event; grow vec if needed
        if id as usize >= self.events.len() {
            self.events.resize_with(id as usize + 1, || None);
        }
        self.events[id as usize] = Some(Event {
            when,
            id,
            handler: Box::new(f),
            cancelled: false,
        });

        CancelToken(id)
    }

    /// Schedule an event after a delay from now.
    pub fn after(&mut self, delay: SimDuration, f: impl FnOnce() + 'static) -> CancelToken {
        self.at(self.now + delay, f)
    }

    /// Cancel a scheduled event.
    pub fn cancel(&mut self, token: CancelToken) {
        self.cancelled.insert(token.0);
        if let Some(event) = self
            .events
            .get_mut(token.0 as usize)
            .and_then(|e| e.as_mut())
        {
            event.cancelled = true;
        }
    }

    /// Process one event. Returns true if an event was processed.
    pub fn step_one(&mut self) -> bool {
        loop {
            let Some(entry) = self.queue.pop() else {
                return false;
            };

            // Skip cancelled events
            if self.cancelled.remove(&entry.id) {
                self.events[entry.id as usize] = None;
                continue;
            }

            let Some(event) = self.events[entry.id as usize].take() else {
                continue;
            };

            if event.cancelled {
                continue;
            }

            self.now = event.when;
            (event.handler)();
            return true;
        }
    }

    /// Process all events until queue is empty.
    pub fn step(&mut self) -> bool {
        if !self.step_one() {
            return false;
        }
        while self.step_one() {}
        true
    }

    /// Process events while predicate returns true.
    pub fn step_while(&mut self, mut f: impl FnMut() -> bool) -> bool {
        let mut ran = false;
        while f() && self.step_one() {
            ran = true;
        }
        ran
    }

    /// Process events until the given time, then advance clock to that time.
    pub fn step_until(&mut self, until: SimTime) -> bool {
        loop {
            // Peek at next event
            let next_time = self.peek_next_time();
            match next_time {
                Some(t) if t <= until => {
                    self.step_one();
                }
                _ => {
                    self.now = until;
                    return !self.queue.is_empty();
                }
            }
        }
    }

    /// Process events for the given duration.
    pub fn step_for(&mut self, amount: SimDuration) -> bool {
        self.step_until(self.now + amount)
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        // Account for cancelled events still in the heap
        self.queue.is_empty() || self.queue.iter().all(|e| self.cancelled.contains(&e.id))
    }

    fn peek_next_time(&self) -> Option<SimTime> {
        self.queue.peek().map(|e| e.when)
    }
}

// ─── HeartbeatTimer ──────────────────────────────────────────────────────────

/// Periodic heartbeat timer for monitoring simulation progress.
///
pub struct HeartbeatTimer {
    pub interval: SimDuration,
    pub beats: u32,
}

impl HeartbeatTimer {
    pub fn new(interval: SimDuration) -> Self {
        Self { interval, beats: 0 }
    }

    /// Schedule the first heartbeat. Subsequent beats are scheduled by the
    /// simulation loop calling `beat()`.
    pub fn start(&mut self, scheduler: &mut Scheduler) {
        let interval = self.interval;
        // We can't capture &mut self in the closure due to borrow rules,
        // so heartbeat tracking is done externally.
        scheduler.after(interval, || {});
        self.beats += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn scheduler_processes_events_in_time_order() {
        let mut sched = Scheduler::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        let l1 = Rc::clone(&log);
        sched.at(Duration::from_millis(300), move || l1.borrow_mut().push(3));
        let l2 = Rc::clone(&log);
        sched.at(Duration::from_millis(100), move || l2.borrow_mut().push(1));
        let l3 = Rc::clone(&log);
        sched.at(Duration::from_millis(200), move || l3.borrow_mut().push(2));

        sched.step();

        assert_eq!(*log.borrow(), vec![1, 2, 3]);
        assert_eq!(sched.now(), Duration::from_millis(300));
    }

    #[test]
    fn scheduler_cancel_prevents_execution() {
        let mut sched = Scheduler::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        let l1 = Rc::clone(&log);
        sched.at(Duration::from_millis(100), move || l1.borrow_mut().push(1));
        let l2 = Rc::clone(&log);
        let token = sched.at(Duration::from_millis(200), move || l2.borrow_mut().push(2));
        let l3 = Rc::clone(&log);
        sched.at(Duration::from_millis(300), move || l3.borrow_mut().push(3));

        sched.cancel(token);
        sched.step();

        assert_eq!(*log.borrow(), vec![1, 3]);
    }

    #[test]
    fn scheduler_step_until_advances_clock() {
        let mut sched = Scheduler::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        let l1 = Rc::clone(&log);
        sched.at(Duration::from_millis(100), move || l1.borrow_mut().push(1));
        let l2 = Rc::clone(&log);
        sched.at(Duration::from_millis(500), move || l2.borrow_mut().push(5));

        sched.step_until(Duration::from_millis(300));

        assert_eq!(*log.borrow(), vec![1]); // only event at 100ms processed
        assert_eq!(sched.now(), Duration::from_millis(300)); // clock at 300ms
    }

    #[test]
    fn scheduler_step_for_processes_within_window() {
        let mut sched = Scheduler::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        let l1 = Rc::clone(&log);
        sched.at(Duration::from_millis(50), move || l1.borrow_mut().push(1));
        let l2 = Rc::clone(&log);
        sched.at(Duration::from_millis(150), move || l2.borrow_mut().push(2));

        sched.step_for(Duration::from_millis(100));

        assert_eq!(*log.borrow(), vec![1]);
        assert_eq!(sched.now(), Duration::from_millis(100));
    }

    #[test]
    fn scheduler_after_schedules_relative_to_now() {
        let mut sched = Scheduler::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        // Advance to 100ms first
        sched.step_until(Duration::from_millis(100));

        let l1 = Rc::clone(&log);
        sched.after(Duration::from_millis(50), move || l1.borrow_mut().push(1));

        sched.step();

        assert_eq!(*log.borrow(), vec![1]);
        assert_eq!(sched.now(), Duration::from_millis(150));
    }

    #[test]
    fn scheduler_step_while_stops_on_predicate() {
        let mut sched = Scheduler::new();
        let count = Rc::new(RefCell::new(0u32));

        for i in 1..=5 {
            let c = Rc::clone(&count);
            sched.at(Duration::from_millis(i * 100), move || *c.borrow_mut() += 1);
        }

        let c = Rc::clone(&count);
        sched.step_while(move || *c.borrow() < 3);

        assert_eq!(*count.borrow(), 3);
        assert_eq!(sched.now(), Duration::from_millis(300));
    }
}
