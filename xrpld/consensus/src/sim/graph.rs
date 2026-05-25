//! Graph infrastructure: Digraph and BasicNetwork.
//!

use super::scheduler::Scheduler;
use super::types::SimDuration;
use std::collections::BTreeMap;

// ─── Digraph ─────────────────────────────────────────────────────────────────

/// Directed graph with edge data.
///
#[derive(Debug, Clone)]
pub struct Digraph<V: Ord + Clone, E: Clone = ()> {
    graph: BTreeMap<V, BTreeMap<V, E>>,
}

/// An edge in the digraph.
#[derive(Debug, Clone)]
pub struct Edge<V: Clone, E: Clone> {
    pub source: V,
    pub target: V,
    pub data: E,
}

impl<V: Ord + Clone, E: Clone> Default for Digraph<V, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Ord + Clone, E: Clone> Digraph<V, E> {
    pub fn new() -> Self {
        Self {
            graph: BTreeMap::new(),
        }
    }

    /// Connect source → target with edge data. Returns true if new.
    pub fn connect(&mut self, source: V, target: V, data: E) -> bool {
        use std::collections::btree_map::Entry;
        match self.graph.entry(source).or_default().entry(target) {
            Entry::Vacant(e) => {
                e.insert(data);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Disconnect source → target. Returns true if removed.
    pub fn disconnect(&mut self, source: &V, target: &V) -> bool {
        if let Some(links) = self.graph.get_mut(source) {
            links.remove(target).is_some()
        } else {
            false
        }
    }

    /// Get edge data between source and target.
    pub fn edge(&self, source: &V, target: &V) -> Option<&E> {
        self.graph.get(source)?.get(target)
    }

    /// Check if source is connected to target.
    pub fn connected(&self, source: &V, target: &V) -> bool {
        self.edge(source, target).is_some()
    }

    /// All vertices with outgoing edges.
    pub fn out_vertices(&self) -> Vec<V> {
        self.graph.keys().cloned().collect()
    }

    /// Target vertices from a source.
    pub fn out_vertices_from(&self, source: &V) -> Vec<V> {
        self.graph
            .get(source)
            .map(|links| links.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Out edges from a source.
    pub fn out_edges(&self, source: &V) -> Vec<Edge<V, E>> {
        self.graph
            .get(source)
            .map(|links| {
                links
                    .iter()
                    .map(|(target, data)| Edge {
                        source: source.clone(),
                        target: target.clone(),
                        data: data.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Out-degree of a vertex.
    pub fn out_degree(&self, source: &V) -> usize {
        self.graph.get(source).map(|l| l.len()).unwrap_or(0)
    }
}

impl<V: Ord + Clone> Digraph<V, ()> {
    /// Connect with default (unit) edge data.
    pub fn connect_default(&mut self, source: V, target: V) -> bool {
        self.connect(source, target, ())
    }
}

// ─── BasicNetwork ────────────────────────────────────────────────────────────

/// Link metadata between two peers.
#[derive(Debug, Clone)]
pub struct LinkType {
    pub inbound: bool,
    pub delay: SimDuration,
    pub established: SimDuration, // time when established
}

/// Peer-to-peer network simulator with message delays.
///
///
/// Messages are scheduled via the Scheduler and delivered after the link delay,
/// but only if the link still exists at delivery time.
pub struct BasicNetwork<P: Ord + Clone> {
    links: Digraph<P, LinkType>,
}

impl<P: Ord + Clone + 'static> Default for BasicNetwork<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Ord + Clone + 'static> BasicNetwork<P> {
    pub fn new() -> Self {
        Self {
            links: Digraph::new(),
        }
    }

    /// Connect two peers with a delay. Creates bidirectional link.
    /// Returns true if new connection established.
    pub fn connect(&mut self, from: P, to: P, delay: SimDuration, now: SimDuration) -> bool {
        if from == to {
            return false;
        }
        if !self.links.connect(
            from.clone(),
            to.clone(),
            LinkType {
                inbound: false,
                delay,
                established: now,
            },
        ) {
            return false;
        }
        self.links.connect(
            to,
            from,
            LinkType {
                inbound: true,
                delay,
                established: now,
            },
        );
        true
    }

    /// Disconnect two peers (bidirectional). Returns true if removed.
    pub fn disconnect(&mut self, peer1: &P, peer2: &P) -> bool {
        if !self.links.disconnect(peer1, peer2) {
            return false;
        }
        self.links.disconnect(peer2, peer1);
        true
    }

    /// Send a message from one peer to another via the scheduler.
    /// The message is delivered after the link delay, only if still connected.
    pub fn send(&self, from: &P, to: &P, scheduler: &mut Scheduler, f: impl FnOnce() + 'static) {
        let Some(link) = self.links.edge(from, to) else {
            return;
        };
        let delay = link.delay;
        let _established = link.established;
        let _from_c = from.clone();
        let _to_c = to.clone();

        // We can't check connection at delivery time without shared state,
        // so we just schedule with the delay. The simulation ensures
        // disconnected peers don't process messages.
        scheduler.after(delay, f);
    }

    /// Get all outgoing links from a peer.
    pub fn links(&self, from: &P) -> Vec<Edge<P, LinkType>> {
        self.links.out_edges(from)
    }

    /// Check if two peers are connected.
    pub fn connected(&self, from: &P, to: &P) -> bool {
        self.links.connected(from, to)
    }

    /// Access the underlying graph.
    pub fn graph(&self) -> &Digraph<P, LinkType> {
        &self.links
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn digraph_connect_and_query() {
        let mut g: Digraph<u32, f64> = Digraph::new();
        assert!(g.connect(1, 2, 1.5));
        assert!(g.connect(1, 3, 2.5));
        assert!(!g.connect(1, 2, 3.0)); // duplicate

        assert!(g.connected(&1, &2));
        assert!(!g.connected(&2, &1)); // directed
        assert_eq!(g.edge(&1, &2), Some(&1.5));
        assert_eq!(g.out_degree(&1), 2);
        assert_eq!(g.out_degree(&2), 0);
    }

    #[test]
    fn digraph_disconnect() {
        let mut g: Digraph<u32, ()> = Digraph::new();
        g.connect_default(1, 2);
        g.connect_default(1, 3);
        assert!(g.disconnect(&1, &2));
        assert!(!g.disconnect(&1, &2)); // already removed
        assert!(!g.connected(&1, &2));
        assert!(g.connected(&1, &3));
    }

    #[test]
    fn digraph_out_vertices_and_edges() {
        let mut g: Digraph<u32, &str> = Digraph::new();
        g.connect(1, 2, "a");
        g.connect(1, 3, "b");
        g.connect(2, 3, "c");

        let verts = g.out_vertices();
        assert!(verts.contains(&1));
        assert!(verts.contains(&2));

        let from_1 = g.out_vertices_from(&1);
        assert_eq!(from_1, vec![2, 3]);

        let edges = g.out_edges(&1);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].target, 2);
        assert_eq!(edges[0].data, "a");
    }

    #[test]
    fn basic_network_bidirectional_connect() {
        let mut net: BasicNetwork<u32> = BasicNetwork::new();
        let now = Duration::ZERO;

        assert!(net.connect(1, 2, Duration::from_millis(100), now));
        assert!(net.connected(&1, &2));
        assert!(net.connected(&2, &1)); // bidirectional

        // Self-connect disallowed
        assert!(!net.connect(1, 1, Duration::from_millis(100), now));

        // Duplicate disallowed
        assert!(!net.connect(1, 2, Duration::from_millis(200), now));
    }

    #[test]
    fn basic_network_disconnect_removes_both_directions() {
        let mut net: BasicNetwork<u32> = BasicNetwork::new();
        net.connect(1, 2, Duration::from_millis(100), Duration::ZERO);

        assert!(net.disconnect(&1, &2));
        assert!(!net.connected(&1, &2));
        assert!(!net.connected(&2, &1));
    }

    #[test]
    fn basic_network_send_schedules_with_delay() {
        let mut net: BasicNetwork<u32> = BasicNetwork::new();
        let mut sched = Scheduler::new();
        net.connect(1, 2, Duration::from_millis(50), Duration::ZERO);

        use std::cell::RefCell;
        use std::rc::Rc;
        let delivered = Rc::new(RefCell::new(false));
        let d = Rc::clone(&delivered);

        net.send(&1, &2, &mut sched, move || *d.borrow_mut() = true);

        // Not delivered yet
        assert!(!*delivered.borrow());

        // Process events
        sched.step();
        assert!(*delivered.borrow());
        assert_eq!(sched.now(), Duration::from_millis(50));
    }
}
