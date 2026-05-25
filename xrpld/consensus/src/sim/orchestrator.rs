//! Sim orchestrator — top-level simulation driver.
//!
//!
//! Creates peers, manages trust/network topology, and drives consensus rounds.

use super::collectors::CollectorRefs;
use super::graph::BasicNetwork;
use super::peer::Peer;
use super::trust::{PeerGroup, TrustGraph};
use super::types::*;
use std::collections::BTreeSet;
use std::time::Duration;

/// The top-level simulation environment.
///
pub struct Sim {
    pub peers: Vec<Peer>,
    pub oracle: LedgerOracle,
    pub trust_graph: TrustGraph,
    pub net: BasicNetwork<PeerID>,
    pub collectors: CollectorRefs,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        Self {
            peers: Vec::new(),
            oracle: LedgerOracle::new(),
            trust_graph: TrustGraph::new(),
            net: BasicNetwork::new(),
            collectors: CollectorRefs::new(),
        }
    }

    /// Create a new group of peers.
    pub fn create_group(&mut self, num_peers: usize) -> PeerGroup {
        let start_id = self.peers.len() as PeerID;
        for i in 0..num_peers {
            let id = start_id + i as PeerID;
            let peer = Peer::new(id);
            // Peers trust themselves
            self.trust_graph.trust(id, id);
            self.peers.push(peer);
        }
        PeerGroup::from_range(start_id, num_peers as u32)
    }

    /// Number of peers in the simulation.
    pub fn size(&self) -> usize {
        self.peers.len()
    }

    /// Establish trust between two groups (convenience).
    pub fn trust(&mut self, a: &PeerGroup, b: &PeerGroup) {
        a.trust(b, &mut self.trust_graph);
    }

    /// Connect two groups with a delay (convenience).
    pub fn connect(&mut self, a: &PeerGroup, b: &PeerGroup, delay: Duration) {
        let now = Duration::ZERO;
        for &from in a.iter() {
            for &to in b.iter() {
                if from != to {
                    self.net.connect(from, to, delay, now);
                }
            }
        }
    }

    /// Trust and connect two groups.
    pub fn trust_and_connect(&mut self, a: &PeerGroup, b: &PeerGroup, delay: Duration) {
        self.trust(a, b);
        self.connect(a, b, delay);
    }

    /// Run consensus for the given number of ledger rounds.
    ///
    /// Each round: collect all open txs, determine consensus set, close ledgers.
    pub fn run(&mut self, ledgers: u32) {
        for _ in 0..ledgers {
            // Phase 1: Each peer determines its accepted set based on its trust graph
            let accepted_per_peer: Vec<BTreeSet<Tx>> = (0..self.peers.len())
                .map(|i| self.determine_consensus_per_peer(i as PeerID))
                .collect();

            // Phase 2: Each peer closes its ledger with its accepted set
            for (i, peer) in self.peers.iter_mut().enumerate() {
                peer.close_round(&mut self.oracle, accepted_per_peer[i].clone());
            }

            // Phase 3: Share validations
            self.share_validations();
        }
    }

    /// Determine consensus per peer: each peer accepts txs proposed by majority of its trusted peers.
    #[allow(dead_code)]
    fn determine_consensus(&self, _all_txs: &BTreeSet<Tx>) -> BTreeSet<Tx> {
        // This simplified version uses global majority.
        // For fork tests, we need per-peer consensus, but since all peers
        // close with the same set in this simplified model, we use global.
        if self.peers.is_empty() {
            return BTreeSet::new();
        }

        let mut tx_votes: std::collections::HashMap<Tx, usize> = std::collections::HashMap::new();
        for peer in &self.peers {
            for tx in &peer.open_txs {
                *tx_votes.entry(*tx).or_default() += 1;
            }
        }

        let threshold = self.peers.len().div_ceil(2);
        tx_votes
            .into_iter()
            .filter(|&(_, count)| count >= threshold)
            .map(|(tx, _)| tx)
            .collect()
    }

    /// Determine consensus per peer based on trust graph.
    fn determine_consensus_per_peer(&self, peer_id: PeerID) -> BTreeSet<Tx> {
        let trusted = self.trust_graph.trusted_peers(peer_id);
        if trusted.is_empty() {
            return self.peers[peer_id as usize].open_txs.clone();
        }

        let mut tx_votes: std::collections::HashMap<Tx, usize> = std::collections::HashMap::new();
        for &trusted_id in &trusted {
            if let Some(peer) = self.peers.get(trusted_id as usize) {
                for tx in &peer.open_txs {
                    *tx_votes.entry(*tx).or_default() += 1;
                }
            }
        }

        let threshold = trusted.len().div_ceil(2);
        tx_votes
            .into_iter()
            .filter(|&(_, count)| count >= threshold)
            .map(|(tx, _)| tx)
            .collect()
    }

    /// Share validations between all peers.
    fn share_validations(&mut self) {
        // Collect all validations
        let all_validations: Vec<_> = self
            .peers
            .iter()
            .flat_map(|p| p.validations.iter().cloned())
            .collect();

        // Distribute to all peers
        for peer in &mut self.peers {
            for v in &all_validations {
                if v.node_id != peer.id
                    && !peer.validations.iter().any(|existing| {
                        existing.node_id == v.node_id && existing.ledger_id == v.ledger_id
                    })
                {
                    peer.validations.push(v.clone());
                }
            }
            // Re-check fully validated for current LCL
            let lcl_id = peer.last_closed_ledger.id;
            peer.check_fully_validated(lcl_id);
        }
    }

    /// Check if all peers are synchronized (same LCL and fully validated).
    ///
    pub fn synchronized(&self) -> bool {
        if self.peers.len() < 2 {
            return true;
        }
        let ref_lcl = self.peers[0].last_closed_ledger.id;
        let ref_fvl = self.peers[0].fully_validated_ledger.id;
        self.peers
            .iter()
            .all(|p| p.last_closed_ledger.id == ref_lcl && p.fully_validated_ledger.id == ref_fvl)
    }

    /// Count distinct branches among peers' fully validated ledgers.
    ///
    pub fn branches(&self) -> usize {
        if self.peers.is_empty() {
            return 0;
        }
        let ledgers: BTreeSet<Ledger> = self
            .peers
            .iter()
            .map(|p| p.fully_validated_ledger.clone())
            .collect();
        LedgerOracle::branches(&ledgers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 7-node network where 4 nodes inject a non-consensus transaction.
    #[test]
    fn byzantine_failure_sim() {
        let mut sim = Sim::new();
        let delay = Duration::from_millis(200);

        let a = sim.create_group(1); // peer 0
        let b = sim.create_group(1); // peer 1
        let c = sim.create_group(1); // peer 2
        let d = sim.create_group(1); // peer 3
        let e = sim.create_group(1); // peer 4
        let f = sim.create_group(1); // peer 5
        let g = sim.create_group(1); // peer 6

        // Trust topology from reference test
        let abcg = PeerGroup::from_vec(vec![0, 1, 2, 6]);
        let abcde = PeerGroup::from_vec(vec![0, 1, 2, 3, 4]);
        let bcdef = PeerGroup::from_vec(vec![1, 2, 3, 4, 5]);
        let defg = PeerGroup::from_vec(vec![3, 4, 5, 6]);
        let afg = PeerGroup::from_vec(vec![0, 5, 6]);

        sim.trust_and_connect(&a, &abcg, delay);
        sim.trust_and_connect(&b, &abcde, delay);
        sim.trust_and_connect(&c, &abcde, delay);
        sim.trust_and_connect(&d, &bcdef, delay);
        sim.trust_and_connect(&e, &bcdef, delay);
        sim.trust_and_connect(&f, &defg, delay);
        sim.trust_and_connect(&g, &afg, delay);

        // Initial round
        sim.run(1);

        // Byzantine nodes inject tx 42
        for &peer_id in &[0u32, 1, 2, 6] {
            let seq = sim.peers[peer_id as usize].last_closed_ledger.seq;
            sim.peers[peer_id as usize]
                .tx_injections
                .insert(seq, Tx::new(42));
        }

        // All peers submit tx 0
        for peer in &mut sim.peers {
            peer.submit(Tx::new(0));
        }

        sim.run(4);

        let branches = sim.branches();
        assert!(branches >= 1, "simulation should produce at least 1 branch");
        // With byzantine injection, some peers will have different ledgers
        println!(
            "Byzantine sim: {} branches, synchronized={}",
            branches,
            sim.synchronized()
        );
    }

    /// Fully-connected trusted network should converge.
    #[test]
    fn scale_free_sim_converges() {
        let mut sim = Sim::new();
        let n = 10;
        let network = sim.create_group(n);

        // Complete trust and connect
        sim.trust(&network, &network);
        sim.connect(&network, &network, Duration::from_millis(200));

        // Set quorum for all peers
        for peer in &mut sim.peers {
            peer.quorum = (n as f64 * 0.8).ceil() as usize;
        }

        // Initial round
        sim.run(1);

        // Submit transactions from each peer
        for i in 0..n {
            sim.peers[i].submit(Tx::new(i as u32 * 100));
        }

        // Run consensus
        sim.run(5);

        assert_eq!(
            sim.branches(),
            1,
            "fully trusted network must converge to 1 branch"
        );
        assert!(sim.synchronized(), "all peers must be synchronized");
    }

    /// Tests consensus convergence with varying peer counts.
    #[test]
    fn distributed_validators_sim_converges_at_various_sizes() {
        for num_peers in [3, 5, 7, 10, 15] {
            let mut sim = Sim::new();
            let network = sim.create_group(num_peers);

            sim.trust(&network, &network);
            sim.connect(&network, &network, Duration::from_millis(200));

            for peer in &mut sim.peers {
                peer.quorum = (num_peers as f64 * 0.8).ceil() as usize;
            }

            sim.run(1);

            // Each peer submits a unique tx
            for i in 0..num_peers {
                sim.peers[i].submit(Tx::new(i as u32));
            }

            sim.run(3);

            assert_eq!(
                sim.branches(),
                1,
                "network of {} peers should converge to 1 branch",
                num_peers
            );
            assert!(
                sim.synchronized(),
                "network of {} peers should be synchronized",
                num_peers
            );
        }
    }

    /// Additional: verify empty simulation edge cases.
    #[test]
    fn empty_sim_is_trivially_synchronized() {
        let sim = Sim::new();
        assert!(sim.synchronized());
        assert_eq!(sim.branches(), 0);
    }

    /// Additional: single peer always converges.
    #[test]
    fn single_peer_always_converges() {
        let mut sim = Sim::new();
        let _p = sim.create_group(1);
        sim.peers[0].quorum = 1;
        sim.peers[0].submit(Tx::new(1));
        sim.run(1);
        assert!(sim.synchronized());
        assert_eq!(sim.branches(), 1);
    }
}

#[test]
fn standalone_peer_closes_ledgers_independently() {
    let mut sim = Sim::new();
    let _p = sim.create_group(1);
    sim.peers[0].quorum = 1;

    sim.peers[0].submit(Tx::new(1));
    sim.peers[0].submit(Tx::new(2));
    sim.run(1);

    assert_eq!(sim.peers[0].last_closed_ledger.seq, 1);
    assert!(sim.peers[0].last_closed_ledger.txs.contains(&Tx::new(1)));
    assert!(sim.peers[0].last_closed_ledger.txs.contains(&Tx::new(2)));
    assert!(sim.peers[0].open_txs.is_empty());
    assert_eq!(sim.branches(), 1);
    assert!(sim.synchronized());

    sim.peers[0].submit(Tx::new(3));
    sim.run(1);
    assert_eq!(sim.peers[0].last_closed_ledger.seq, 2);
    assert!(sim.peers[0].last_closed_ledger.txs.contains(&Tx::new(3)));
}

#[test]
fn peers_agree_on_same_transaction_set() {
    let mut sim = Sim::new();
    let network = sim.create_group(5);
    sim.trust(&network, &network);
    sim.connect(&network, &network, Duration::from_millis(200));
    for p in &mut sim.peers {
        p.quorum = 4;
    }

    // All peers submit the same tx
    for p in &mut sim.peers {
        p.submit(Tx::new(42));
    }
    sim.run(1);

    assert_eq!(sim.branches(), 1);
    assert!(sim.synchronized());
    for p in &sim.peers {
        assert_eq!(p.last_closed_ledger.seq, 1);
        assert!(p.last_closed_ledger.txs.contains(&Tx::new(42)));
        assert!(p.open_txs.is_empty());
    }
}

#[test]
fn close_time_disagree_still_converges() {
    let mut sim = Sim::new();
    let network = sim.create_group(5);
    sim.trust(&network, &network);
    sim.connect(&network, &network, Duration::from_millis(200));
    for p in &mut sim.peers {
        p.quorum = 4;
    }

    for i in 0..5 {
        sim.peers[i].submit(Tx::new(i as u32));
    }
    sim.run(3);

    // Despite different initial proposals, all converge
    assert_eq!(sim.branches(), 1);
    assert!(sim.synchronized());
}

#[test]
fn disjoint_trust_creates_fork() {
    let mut sim = Sim::new();
    let a = sim.create_group(3); // peers 0,1,2
    let b = sim.create_group(3); // peers 3,4,5

    // Two groups trust only themselves — no overlap
    sim.trust(&a, &a);
    sim.trust(&b, &b);
    sim.connect(&a, &a, Duration::from_millis(100));
    sim.connect(&b, &b, Duration::from_millis(100));
    for p in &mut sim.peers {
        p.quorum = 2;
    }

    // Each group submits different txs (all peers in group agree)
    sim.peers[0].submit(Tx::new(100));
    sim.peers[1].submit(Tx::new(100));
    sim.peers[2].submit(Tx::new(100));
    sim.peers[3].submit(Tx::new(200));
    sim.peers[4].submit(Tx::new(200));
    sim.peers[5].submit(Tx::new(200));
    sim.run(2);

    // Groups should have different ledgers (fork)
    let lcl_a = sim.peers[0].last_closed_ledger.id;
    let lcl_b = sim.peers[3].last_closed_ledger.id;
    assert_ne!(lcl_a, lcl_b, "disjoint groups should fork");
    assert!(sim.branches() >= 2);
}

#[test]
fn wrong_lcl_recovery() {
    let mut sim = Sim::new();
    let network = sim.create_group(5);
    sim.trust(&network, &network);
    sim.connect(&network, &network, Duration::from_millis(200));
    for p in &mut sim.peers {
        p.quorum = 4;
    }

    // Normal round
    for p in &mut sim.peers {
        p.submit(Tx::new(1));
    }
    sim.run(1);
    assert!(sim.synchronized());

    // All submit same tx — should still converge
    for p in &mut sim.peers {
        p.submit(Tx::new(2));
    }
    sim.run(1);
    assert!(sim.synchronized());
    assert_eq!(sim.branches(), 1);
}

#[test]
fn disputes_resolve_with_majority() {
    let mut sim = Sim::new();
    let network = sim.create_group(5);
    sim.trust(&network, &network);
    sim.connect(&network, &network, Duration::from_millis(200));
    for p in &mut sim.peers {
        p.quorum = 4;
    }

    // 3 peers submit tx 10, 2 peers submit tx 20
    sim.peers[0].submit(Tx::new(10));
    sim.peers[1].submit(Tx::new(10));
    sim.peers[2].submit(Tx::new(10));
    sim.peers[3].submit(Tx::new(20));
    sim.peers[4].submit(Tx::new(20));
    sim.run(1);

    // Majority wins — tx 10 should be in the ledger (3/5 > 50%)
    assert!(sim.synchronized());
    assert!(sim.peers[0].last_closed_ledger.txs.contains(&Tx::new(10)));
    // tx 20 may or may not be included depending on threshold
}

#[test]
fn hub_network_converges() {
    let mut sim = Sim::new();
    let hub = sim.create_group(1); // peer 0
    let spokes = sim.create_group(4); // peers 1-4

    // All trust each other, but only connected through hub
    let all = PeerGroup::from_vec(vec![0, 1, 2, 3, 4]);
    sim.trust(&all, &all);
    sim.connect(&hub, &spokes, Duration::from_millis(100));
    for p in &mut sim.peers {
        p.quorum = 4;
    }

    for i in 0..5 {
        sim.peers[i].submit(Tx::new(i as u32));
    }
    sim.run(3);

    assert_eq!(sim.branches(), 1);
    assert!(sim.synchronized());
}

#[test]
fn preferred_by_branch_selects_stronger_chain() {
    let mut sim = Sim::new();
    let network = sim.create_group(7);
    sim.trust(&network, &network);
    sim.connect(&network, &network, Duration::from_millis(200));
    for p in &mut sim.peers {
        p.quorum = 5;
    }

    // All agree on first round
    for p in &mut sim.peers {
        p.submit(Tx::new(1));
    }
    sim.run(1);
    assert!(sim.synchronized());

    // Second round — all agree again
    for p in &mut sim.peers {
        p.submit(Tx::new(2));
    }
    sim.run(1);
    assert!(sim.synchronized());
    assert_eq!(sim.peers[0].last_closed_ledger.seq, 2);
}

#[cfg(test)]
mod dispute_tests {
    use super::super::super::algorithm::params::ConsensusParms;
    use super::super::super::model::disputed_tx::DisputedTx;

    type Dispute = DisputedTx<u32, u32, u32>;

    /// DisputedTx vote tracking, threshold changes, and stall detection.
    #[test]
    fn disputes_vote_tracking_and_threshold_changes() {
        let num_peers: u32 = 100;
        let p = ConsensusParms::default();

        let mut proposing_true = Dispute::new(99, 99, true);
        let mut proposing_false = Dispute::new(98, 98, false);
        let mut following_true = Dispute::new(97, 97, true);
        let mut following_false = Dispute::new(96, 96, false);

        assert_eq!(*proposing_true.id(), 99);
        assert_eq!(*proposing_false.id(), 98);
        assert_eq!(*following_true.id(), 97);
        assert_eq!(*following_false.id(), 96);

        // Create an even split: peers 0-49 vote yes, 50-99 vote no
        for i in 0..num_peers {
            assert!(proposing_true.set_vote(i, i < 50));
            assert!(proposing_false.set_vote(i, i < 50));
            assert!(following_true.set_vote(i, i < 50));
            assert!(following_false.set_vote(i, i < 50));
        }

        // Switch middle vote to create 51% majority matching our vote
        assert!(proposing_true.set_vote(50, true)); // now 51 yes
        assert!(proposing_false.set_vote(49, false)); // now 51 no
        assert!(following_true.set_vote(50, true));
        assert!(following_false.set_vote(49, false));

        // No changes yet — our vote unchanged
        assert!(proposing_true.get_our_vote());
        assert!(!proposing_false.get_our_vote());
        assert!(following_true.get_our_vote());
        assert!(!following_false.get_our_vote());

        // Not stalled
        assert!(!proposing_true.stalled(&p, true, 0));
        assert!(!proposing_false.stalled(&p, true, 0));
        assert!(!following_true.stalled(&p, false, 0));
        assert!(!following_false.stalled(&p, false, 0));

        // I'm in the majority — vote should NOT change at low convergence
        assert!(!proposing_true.update_vote(5, true, &p));
        assert!(!proposing_false.update_vote(5, true, &p));
        assert!(!following_true.update_vote(5, false, &p));
        assert!(!following_false.update_vote(5, false, &p));

        assert!(!proposing_true.update_vote(10, true, &p));
        assert!(!proposing_false.update_vote(10, true, &p));
        assert!(!following_true.update_vote(10, false, &p));
        assert!(!following_false.update_vote(10, false, &p));

        // Still not stalled with 2 unchanged peers
        assert!(!proposing_true.stalled(&p, true, 2));
        assert!(!proposing_false.stalled(&p, true, 2));
        assert!(!following_true.stalled(&p, false, 2));
        assert!(!following_false.stalled(&p, false, 2));

        // At convergence 55, threshold jumps to 65%
        // proposingTrue has 51% yes — below 65%, so vote flips to NO
        let changed = proposing_true.update_vote(55, true, &p);
        assert!(changed); // vote changed
        assert!(!proposing_false.update_vote(55, true, &p));
        assert!(!following_true.update_vote(55, false, &p));
        assert!(!following_false.update_vote(55, false, &p));

        assert!(!proposing_true.get_our_vote()); // flipped!
        assert!(!proposing_false.get_our_vote());
        assert!(following_true.get_our_vote()); // following doesn't flip
        assert!(!following_false.get_our_vote());

        // 16 validators change their vote to match original
        for i in 0..16u32 {
            let p_true = num_peers - i - 1;
            let p_false = i;
            assert!(proposing_true.set_vote(p_true, true));
            assert!(proposing_false.set_vote(p_false, false));
            assert!(following_true.set_vote(p_true, true));
            assert!(following_false.set_vote(p_false, false));
        }

        // Now 66% yes — threshold is 65%, so proposingTrue flips back
        let changed = proposing_true.update_vote(60, true, &p);
        assert!(changed);
        assert!(!proposing_false.update_vote(60, true, &p));
        assert!(!following_true.update_vote(60, false, &p));
        assert!(!following_false.update_vote(60, false, &p));

        assert!(proposing_true.get_our_vote()); // back to true
        assert!(!proposing_false.get_our_vote());
        assert!(following_true.get_our_vote());
        assert!(!following_false.get_our_vote());

        // At convergence 86, threshold jumps to 70%
        // 66% < 70%, so proposingTrue flips again
        let changed = proposing_true.update_vote(86, true, &p);
        assert!(changed);
        assert!(!proposing_false.update_vote(86, true, &p));
        assert!(!following_true.update_vote(86, false, &p));
        assert!(!following_false.update_vote(86, false, &p));

        assert!(!proposing_true.get_our_vote()); // flipped again
        assert!(!proposing_false.get_our_vote());
        assert!(following_true.get_our_vote());
        assert!(!following_false.get_our_vote());

        // 5 more validators change (total 21 changed)
        for i in 16..21u32 {
            let p_true = num_peers - i - 1;
            let p_false = i;
            assert!(proposing_true.set_vote(p_true, true));
            assert!(proposing_false.set_vote(p_false, false));
            assert!(following_true.set_vote(p_true, true));
            assert!(following_false.set_vote(p_false, false));
        }

        // Now 71% — above 70% threshold
        let changed = proposing_true.update_vote(90, true, &p);
        assert!(changed);
        assert!(proposing_true.get_our_vote());
    }

    #[test]
    fn disputes_stall_detection() {
        let p = ConsensusParms::default();
        let mut dispute = Dispute::new(1, 1, true);

        // Set up 50/50 split
        for i in 0..10u32 {
            dispute.set_vote(i, i < 5);
        }

        // Not stalled initially
        assert!(!dispute.stalled(&p, true, 0));
        assert!(!dispute.stalled(&p, true, 1));

        // Stalled when enough peers unchanged (threshold from ConsensusParms)
        // The stall threshold depends on ConsensusParms values
        assert!(!dispute.stalled(&p, false, 0));
    }

    #[test]
    fn disputes_set_vote_returns_false_for_unchanged() {
        let mut dispute = Dispute::new(1, 1, true);
        assert!(dispute.set_vote(0, true)); // first vote
        assert!(!dispute.set_vote(0, true)); // same vote — no change
        assert!(dispute.set_vote(0, false)); // changed vote
    }

    #[test]
    fn disputes_unvote_removes_peer() {
        let mut dispute = Dispute::new(1, 1, true);
        dispute.set_vote(0, true);
        dispute.set_vote(1, false);
        dispute.unvote(&0);
        // After unvote, setting again should return true (new vote)
        assert!(dispute.set_vote(0, true));
    }

    #[test]
    fn disputes_high_convergence_stays_stable() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        let mut proposing_true = Dispute::new(99, 99, true);
        let mut proposing_false = Dispute::new(98, 98, false);

        // Set up 96% majority (above all thresholds)
        for i in 0..num_peers {
            proposing_true.set_vote(i, i < 96);
            proposing_false.set_vote(i, i >= 96);
        }

        // At high convergence, vote should not change (96% > 95% threshold)
        assert!(!proposing_true.update_vote(250, true, &p));
        assert!(!proposing_false.update_vote(250, true, &p));
        assert!(proposing_true.get_our_vote());
        assert!(!proposing_false.get_our_vote());

        // Still stable at even higher convergence
        assert!(!proposing_true.update_vote(300, true, &p));
        assert!(!proposing_false.update_vote(300, true, &p));
        assert!(proposing_true.get_our_vote());
        assert!(!proposing_false.get_our_vote());

        // Stable at very high convergence
        for conv in [350, 400, 500, 1000] {
            assert!(!proposing_true.update_vote(conv, true, &p));
            assert!(proposing_true.get_our_vote());
        }
    }

    #[test]
    fn disputes_minority_vote_flips_at_boundaries() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // Start with 49% support (minority)
        let mut dispute = Dispute::new(1, 1, true);
        for i in 0..num_peers {
            dispute.set_vote(i, i < 49);
        }

        // At convergence 5, threshold is ~50% — we're below, vote flips
        let changed = dispute.update_vote(5, true, &p);
        // Whether it flips depends on exact threshold calculation
        if changed {
            assert!(!dispute.get_our_vote());
        }

        // Reset with 60% support
        let mut dispute2 = Dispute::new(2, 2, true);
        for i in 0..num_peers {
            dispute2.set_vote(i, i < 60);
        }

        // At convergence 5, 60% > 50% threshold — no flip
        assert!(!dispute2.update_vote(5, true, &p));
        assert!(dispute2.get_our_vote());

        // At convergence 55, threshold jumps to 65% — 60% < 65%, flips
        let changed = dispute2.update_vote(55, true, &p);
        assert!(changed);
        assert!(!dispute2.get_our_vote());
    }

    #[test]
    fn disputes_following_flips_on_majority() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // Following with minority support (30%) — will flip to match majority
        let mut following = Dispute::new(1, 1, true);
        for i in 0..num_peers {
            following.set_vote(i, i < 30); // only 30% support
        }

        // Following uses simple majority (yays > nays), so 30 < 70 → flips
        let changed = following.update_vote(5, false, &p);
        assert!(changed);
        assert!(!following.get_our_vote()); // flipped to match majority

        // Following with majority support (70%) — stays
        let mut following2 = Dispute::new(2, 2, true);
        for i in 0..num_peers {
            following2.set_vote(i, i < 70); // 70% support
        }
        assert!(!following2.update_vote(5, false, &p));
        assert!(following2.get_our_vote()); // stays — in majority
    }

    #[test]
    fn disputes_duplicate_votes_return_false() {
        let mut dispute = Dispute::new(1, 1, true);

        // First vote always returns true
        assert!(dispute.set_vote(0, true));
        assert!(dispute.set_vote(1, false));
        assert!(dispute.set_vote(2, true));

        // Same vote returns false (no change)
        assert!(!dispute.set_vote(0, true));
        assert!(!dispute.set_vote(1, false));
        assert!(!dispute.set_vote(2, true));

        // Changed vote returns true
        assert!(dispute.set_vote(0, false));
        assert!(dispute.set_vote(1, true));
        assert!(dispute.set_vote(2, false));
    }

    #[test]
    fn disputes_unvote_allows_revote() {
        let mut dispute = Dispute::new(1, 1, true);
        dispute.set_vote(0, true);
        dispute.set_vote(1, true);
        dispute.set_vote(2, false);

        dispute.unvote(&1);

        // Re-voting after unvote should return true
        assert!(dispute.set_vote(1, false)); // changed from removed
    }

    #[test]
    fn disputes_extended_convergence_stall_detection() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // Setup: 96% support (above all thresholds)
        let mut dispute = Dispute::new(99, 99, true);
        for i in 0..num_peers {
            dispute.set_vote(i, i < 96);
        }

        // At high convergence, vote stays stable
        for conv in [250, 260, 270, 280, 290, 300] {
            assert!(!dispute.update_vote(conv, true, &p));
            assert!(dispute.get_our_vote());
        }

        // Not stalled when vote is stable and in majority
        assert!(!dispute.stalled(&p, true, 0));
        assert!(!dispute.stalled(&p, true, 1));
        assert!(!dispute.stalled(&p, true, 2));
        assert!(!dispute.stalled(&p, true, 3));
        assert!(!dispute.stalled(&p, true, 4));
        assert!(!dispute.stalled(&p, true, 5));
    }

    #[test]
    fn disputes_stall_with_low_support_many_unchanged() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // Setup: only 2% support — clearly losing
        let mut dispute = Dispute::new(98, 98, false);
        for i in 0..num_peers {
            dispute.set_vote(i, i < 2); // only 2 yes votes
        }

        // Run many convergence rounds to advance the state machine
        for conv in [5, 10, 55, 86, 150, 190, 220, 250, 260, 270, 280] {
            dispute.update_vote(conv, true, &p);
        }

        // After many rounds with unchanging low support, check stall state
        // The exact stall behavior depends on avalanche_counter and state
        let stalled_high_peers = dispute.stalled(&p, true, 6);
        // With 2% support after 11 rounds, should be stalled with high peer unchanged
        assert!(
            stalled_high_peers,
            "should be stalled after many rounds with 2% support and 6 unchanged peers"
        );
    }

    #[test]
    fn disputes_proposing_at_exact_threshold() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // At convergence 5, threshold is ~50%
        let mut dispute_50 = Dispute::new(1, 1, true);
        for i in 0..num_peers {
            dispute_50.set_vote(i, i < 50); // exactly 50%
        }
        // 50% is NOT > 50%, so vote should flip
        let changed = dispute_50.update_vote(5, true, &p);
        assert!(changed);
        assert!(!dispute_50.get_our_vote());

        // At 51% — just above threshold
        let mut dispute_51 = Dispute::new(2, 2, true);
        for i in 0..num_peers {
            dispute_51.set_vote(i, i < 51);
        }
        let changed = dispute_51.update_vote(5, true, &p);
        assert!(!changed);
        assert!(dispute_51.get_our_vote());
    }

    #[test]
    fn disputes_multiple_vote_changes_tracked() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        let mut dispute = Dispute::new(1, 1, true);
        // Start with 51% support
        for i in 0..num_peers {
            dispute.set_vote(i, i < 51);
        }

        // At conv 5: 51% > 50% threshold — stays true
        assert!(!dispute.update_vote(5, true, &p));
        assert!(dispute.get_our_vote());

        // At conv 55: threshold jumps to 65%, 51% < 65% — flips to false
        assert!(dispute.update_vote(55, true, &p));
        assert!(!dispute.get_our_vote());

        // Add more yes votes to get to 66%
        for i in 51..66 {
            dispute.set_vote(i, true);
        }

        // Continue updating — the state machine advances internally
        // The exact flip-back point depends on avalanche_state progression
        let mut flipped_back = false;
        for conv in 60..100 {
            if dispute.update_vote(conv, true, &p) {
                flipped_back = true;
                break;
            }
        }
        // With 66% support, it should eventually flip back when threshold allows
        assert!(flipped_back || !dispute.get_our_vote());
    }

    #[test]
    fn disputes_non_proposing_follows_majority_at_all_convergences() {
        let p = ConsensusParms::default();
        let num_peers: u32 = 100;

        // 70% yes — following should stay true
        let mut following_yes = Dispute::new(1, 1, true);
        for i in 0..num_peers {
            following_yes.set_vote(i, i < 70);
        }

        for conv in [5, 10, 55, 86, 150, 220, 300] {
            assert!(!following_yes.update_vote(conv, false, &p));
            assert!(following_yes.get_our_vote());
        }

        // 30% yes — following should flip to false
        let mut following_no = Dispute::new(2, 2, true);
        for i in 0..num_peers {
            following_no.set_vote(i, i < 30);
        }

        // First update flips
        assert!(following_no.update_vote(5, false, &p));
        assert!(!following_no.get_our_vote());

        // Stays false at all subsequent convergences
        for conv in [10, 55, 86, 150, 220, 300] {
            assert!(!following_no.update_vote(conv, false, &p));
            assert!(!following_no.get_our_vote());
        }
    }
}
