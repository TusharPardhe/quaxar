//! Trust graph and peer group management.
//!

use super::graph::Digraph;
use super::types::{PeerID, SimDuration};
use std::collections::{BTreeSet, HashSet};

// ─── TrustGraph ──────────────────────────────────────────────────────────────

/// Directed trust graph. If peer A trusts peer B, then B is in A's UNL.
///
pub struct TrustGraph {
    graph: Digraph<PeerID, ()>,
}

/// Information about a pair of UNLs that can fork.
#[derive(Debug, Clone)]
pub struct ForkInfo {
    pub unl_a: BTreeSet<PeerID>,
    pub unl_b: BTreeSet<PeerID>,
    pub overlap: usize,
    pub required: f64,
}

impl Default for TrustGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustGraph {
    pub fn new() -> Self {
        Self {
            graph: Digraph::new(),
        }
    }

    /// Establish trust: `from` puts `to` in its UNL.
    pub fn trust(&mut self, from: PeerID, to: PeerID) {
        self.graph.connect_default(from, to);
    }

    /// Revoke trust.
    pub fn untrust(&mut self, from: PeerID, to: PeerID) {
        self.graph.disconnect(&from, &to);
    }

    /// Check if `from` trusts `to`.
    pub fn trusts(&self, from: PeerID, to: PeerID) -> bool {
        self.graph.connected(&from, &to)
    }

    /// Get the set of peers trusted by `peer` (its UNL).
    pub fn trusted_peers(&self, peer: PeerID) -> Vec<PeerID> {
        self.graph.out_vertices_from(&peer)
    }

    /// Find pairs of UNLs that violate the no-forking condition.
    pub fn forkable_pairs(&self, quorum: f64) -> Vec<ForkInfo> {
        // Collect unique UNLs
        let mut unique_unls: Vec<BTreeSet<PeerID>> = Vec::new();
        let mut seen: HashSet<Vec<PeerID>> = HashSet::new();

        for peer in self.graph.out_vertices() {
            let mut unl: Vec<PeerID> = self.trusted_peers(peer);
            unl.sort();
            if seen.insert(unl.clone()) {
                unique_unls.push(unl.into_iter().collect());
            }
        }

        let mut res = Vec::new();
        for i in 0..unique_unls.len() {
            for j in (i + 1)..unique_unls.len() {
                let unl_a = &unique_unls[i];
                let unl_b = &unique_unls[j];
                let rhs = 2.0 * (1.0 - quorum) * unl_a.len().max(unl_b.len()) as f64;
                let intersection_size = unl_a.intersection(unl_b).count();

                if (intersection_size as f64) < rhs {
                    res.push(ForkInfo {
                        unl_a: unl_a.clone(),
                        unl_b: unl_b.clone(),
                        overlap: intersection_size,
                        required: rhs,
                    });
                }
            }
        }
        res
    }

    /// Check if this trust graph can fork at the given quorum.
    pub fn can_fork(&self, quorum: f64) -> bool {
        !self.forkable_pairs(quorum).is_empty()
    }
}

// ─── PeerGroup ───────────────────────────────────────────────────────────────

/// A group of peer IDs for bulk trust/connect operations.
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerGroup {
    pub peers: Vec<PeerID>,
}

impl Default for PeerGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerGroup {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    pub fn from_single(peer: PeerID) -> Self {
        Self { peers: vec![peer] }
    }

    pub fn from_vec(mut peers: Vec<PeerID>) -> Self {
        peers.sort();
        peers.dedup();
        Self { peers }
    }

    pub fn from_range(start: PeerID, count: u32) -> Self {
        Self {
            peers: (start..start + count).collect(),
        }
    }

    pub fn contains(&self, peer: PeerID) -> bool {
        self.peers.contains(&peer)
    }

    pub fn size(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PeerID> {
        self.peers.iter()
    }

    /// Establish trust from all peers in self to all peers in `other`.
    pub fn trust(&self, other: &PeerGroup, trust_graph: &mut TrustGraph) {
        for &from in &self.peers {
            for &to in &other.peers {
                trust_graph.trust(from, to);
            }
        }
    }

    /// Revoke trust from all peers in self to all peers in `other`.
    pub fn untrust(&self, other: &PeerGroup, trust_graph: &mut TrustGraph) {
        for &from in &self.peers {
            for &to in &other.peers {
                trust_graph.untrust(from, to);
            }
        }
    }

    /// Establish trust AND connect from all in self to all in `other`.
    pub fn trust_and_connect(
        &self,
        other: &PeerGroup,
        trust_graph: &mut TrustGraph,
        delay: SimDuration,
        connect_fn: &mut dyn FnMut(PeerID, PeerID, SimDuration),
    ) {
        self.trust(other, trust_graph);
        self.connect(other, delay, connect_fn);
    }

    /// Establish network connections from all in self to all in `other`.
    pub fn connect(
        &self,
        other: &PeerGroup,
        delay: SimDuration,
        connect_fn: &mut dyn FnMut(PeerID, PeerID, SimDuration),
    ) {
        for &from in &self.peers {
            for &to in &other.peers {
                if from != to {
                    connect_fn(from, to, delay);
                }
            }
        }
    }

    /// Union of two peer groups.
    pub fn union(&self, other: &PeerGroup) -> PeerGroup {
        let mut combined: BTreeSet<PeerID> = self.peers.iter().copied().collect();
        combined.extend(other.peers.iter());
        PeerGroup {
            peers: combined.into_iter().collect(),
        }
    }

    /// Set difference (self - other).
    pub fn difference(&self, other: &PeerGroup) -> PeerGroup {
        let other_set: BTreeSet<PeerID> = other.peers.iter().copied().collect();
        PeerGroup {
            peers: self
                .peers
                .iter()
                .filter(|p| !other_set.contains(p))
                .copied()
                .collect(),
        }
    }
}

impl std::ops::Add for &PeerGroup {
    type Output = PeerGroup;
    fn add(self, rhs: Self) -> PeerGroup {
        self.union(rhs)
    }
}

impl std::ops::Sub for &PeerGroup {
    type Output = PeerGroup;
    fn sub(self, rhs: Self) -> PeerGroup {
        self.difference(rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn trust_graph_basic_operations() {
        let mut tg = TrustGraph::new();
        tg.trust(1, 2);
        tg.trust(1, 3);
        tg.trust(2, 1);

        assert!(tg.trusts(1, 2));
        assert!(tg.trusts(1, 3));
        assert!(tg.trusts(2, 1));
        assert!(!tg.trusts(3, 1));

        let trusted = tg.trusted_peers(1);
        assert_eq!(trusted.len(), 2);
        assert!(trusted.contains(&2));
        assert!(trusted.contains(&3));

        tg.untrust(1, 2);
        assert!(!tg.trusts(1, 2));
    }

    #[test]
    fn trust_graph_fork_detection() {
        let mut tg = TrustGraph::new();
        // Two completely disjoint UNLs → can fork
        tg.trust(1, 1);
        tg.trust(1, 2);
        tg.trust(1, 3);
        tg.trust(4, 4);
        tg.trust(4, 5);
        tg.trust(4, 6);

        assert!(tg.can_fork(0.8));

        // Complete overlap → cannot fork
        let mut tg2 = TrustGraph::new();
        tg2.trust(1, 1);
        tg2.trust(1, 2);
        tg2.trust(1, 3);
        tg2.trust(2, 1);
        tg2.trust(2, 2);
        tg2.trust(2, 3);
        assert!(!tg2.can_fork(0.8));
    }

    #[test]
    fn peer_group_union_and_difference() {
        let a = PeerGroup::from_vec(vec![1, 2, 3]);
        let b = PeerGroup::from_vec(vec![2, 3, 4]);

        let union = &a + &b;
        assert_eq!(union.peers, vec![1, 2, 3, 4]);

        let diff = &a - &b;
        assert_eq!(diff.peers, vec![1]);
    }

    #[test]
    fn peer_group_trust_establishes_all_pairs() {
        let mut tg = TrustGraph::new();
        let a = PeerGroup::from_vec(vec![1, 2]);
        let b = PeerGroup::from_vec(vec![3, 4]);

        a.trust(&b, &mut tg);

        assert!(tg.trusts(1, 3));
        assert!(tg.trusts(1, 4));
        assert!(tg.trusts(2, 3));
        assert!(tg.trusts(2, 4));
        assert!(!tg.trusts(3, 1)); // not bidirectional
    }

    #[test]
    fn peer_group_connect_calls_connect_fn() {
        let a = PeerGroup::from_vec(vec![1, 2]);
        let b = PeerGroup::from_vec(vec![2, 3]);
        let mut connections = Vec::new();

        a.connect(&b, Duration::from_millis(100), &mut |from, to, _delay| {
            connections.push((from, to));
        });

        // 1→2, 1→3, 2→3 (2→2 skipped as self-connect)
        assert!(connections.contains(&(1, 2)));
        assert!(connections.contains(&(1, 3)));
        assert!(connections.contains(&(2, 3)));
        assert!(!connections.contains(&(2, 2))); // no self-connect
    }

    #[test]
    fn peer_group_from_range() {
        let g = PeerGroup::from_range(5, 3);
        assert_eq!(g.peers, vec![5, 6, 7]);
        assert_eq!(g.size(), 3);
        assert!(g.contains(5));
        assert!(!g.contains(4));
    }
}
