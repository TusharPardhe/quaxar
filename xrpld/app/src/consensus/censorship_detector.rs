//! Censorship detection for transactions repeatedly failing to get into validated ledgers.
//!

/// A tracked transaction with its first-seen sequence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxIDSeq<TxID: Ord, Sequence: Ord> {
    pub txid: TxID,
    pub seq: Sequence,
}

/// Tracks transactions that repeatedly fail to get into validated ledgers.
pub struct CensorshipDetector<TxID: Ord + Clone, Sequence: Ord + Clone> {
    tracker: Vec<TxIDSeq<TxID, Sequence>>,
}

impl<TxID: Ord + Clone, Sequence: Ord + Clone> Default for CensorshipDetector<TxID, Sequence> {
    fn default() -> Self {
        Self::new()
    }
}

impl<TxID: Ord + Clone, Sequence: Ord + Clone> CensorshipDetector<TxID, Sequence> {
    pub fn new() -> Self {
        Self {
            tracker: Vec::new(),
        }
    }

    /// Add transactions being proposed for the current consensus round.
    /// Keeps only items in BOTH old tracker and new proposed (intersection),
    /// preserving old seq values. New items not in old tracker are added fresh.
    pub fn propose(&mut self, mut proposed: Vec<TxIDSeq<TxID, Sequence>>) {
        proposed.sort();
        let old = std::mem::take(&mut self.tracker);
        let mut oi = 0;
        let mut pi = 0;

        // Update proposed entries with old seq values where they intersect
        while oi < old.len() && pi < proposed.len() {
            match old[oi].txid.cmp(&proposed[pi].txid) {
                std::cmp::Ordering::Less => oi += 1,
                std::cmp::Ordering::Greater => pi += 1,
                std::cmp::Ordering::Equal => {
                    // Preserve old sequence value
                    proposed[pi].seq = old[oi].seq.clone();
                    oi += 1;
                    pi += 1;
                }
            }
        }
        self.tracker = proposed;
    }

    /// Remove accepted transactions and those matching the predicate.
    pub fn check(
        &mut self,
        mut accepted: Vec<TxID>,
        mut pred: impl FnMut(&TxID, &Sequence) -> bool,
    ) {
        accepted.sort();
        self.tracker.retain(|entry| {
            if accepted.binary_search(&entry.txid).is_ok() {
                return false;
            }
            if pred(&entry.txid, &entry.seq) {
                return false;
            }
            true
        });
    }

    /// Clear all tracked entries.
    pub fn reset(&mut self) {
        self.tracker.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_preserves_old_seq() {
        let mut det = CensorshipDetector::<u32, u32>::new();
        det.propose(vec![
            TxIDSeq { txid: 1, seq: 100 },
            TxIDSeq { txid: 2, seq: 100 },
        ]);
        // Re-propose with new seq values; old ones should be preserved
        det.propose(vec![
            TxIDSeq { txid: 1, seq: 200 },
            TxIDSeq { txid: 3, seq: 200 },
        ]);
        assert_eq!(det.tracker.len(), 2);
        assert_eq!(det.tracker[0], TxIDSeq { txid: 1, seq: 100 }); // preserved
        assert_eq!(det.tracker[1], TxIDSeq { txid: 3, seq: 200 }); // new
    }

    #[test]
    fn check_removes_accepted_and_predicate() {
        let mut det = CensorshipDetector::<u32, u32>::new();
        det.propose(vec![
            TxIDSeq { txid: 1, seq: 10 },
            TxIDSeq { txid: 2, seq: 10 },
            TxIDSeq { txid: 3, seq: 10 },
        ]);
        det.check(vec![1], |_txid, _seq| false);
        assert_eq!(det.tracker.len(), 2);
        det.check(vec![], |_txid, seq| *seq < 11);
        assert_eq!(det.tracker.len(), 0);
    }

    #[test]
    fn reset_clears() {
        let mut det = CensorshipDetector::<u32, u32>::new();
        det.propose(vec![TxIDSeq { txid: 1, seq: 1 }]);
        det.reset();
        assert!(det.tracker.is_empty());
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    /// The reference test predicate removes items in `remove` and verifies items in `remain`.
    #[test]
    fn censorship_detector_full_sequence_matches_cpp() {
        let mut cdet = CensorshipDetector::<i32, i32>::new();
        let mut round = 0i32;

        let mut do_round = |cdet: &mut CensorshipDetector<i32, i32>,
                            proposed: Vec<i32>,
                            accepted: Vec<i32>,
                            expected_remain: Vec<i32>,
                            to_remove: Vec<i32>| {
            round += 1;
            let proposed_items: Vec<TxIDSeq<i32, i32>> = proposed
                .iter()
                .map(|&id| TxIDSeq {
                    txid: id,
                    seq: round,
                })
                .collect();
            cdet.propose(proposed_items);

            let mut remain_check = expected_remain.clone();
            cdet.check(accepted, |id, _seq| {
                if to_remove.contains(id) {
                    return true;
                }
                if let Some(pos) = remain_check.iter().position(|x| x == id) {
                    remain_check.remove(pos);
                }
                false
            });

            // All expected remain items should have been found in tracker
            assert!(
                remain_check.is_empty(),
                "round {round}: not all remain items found in tracker, missing: {:?}",
                remain_check
            );
        };

        do_round(&mut cdet, vec![], vec![], vec![], vec![]);
        do_round(
            &mut cdet,
            vec![10, 11, 12, 13],
            vec![11, 2],
            vec![10, 13],
            vec![],
        );
        do_round(
            &mut cdet,
            vec![10, 13, 14, 15],
            vec![14],
            vec![10, 13, 15],
            vec![],
        );
        do_round(
            &mut cdet,
            vec![10, 13, 15, 16],
            vec![15, 16],
            vec![10, 13],
            vec![],
        );
        do_round(&mut cdet, vec![10, 13], vec![17, 18], vec![10, 13], vec![]);
        do_round(&mut cdet, vec![10, 19], vec![], vec![10, 19], vec![]);
        do_round(&mut cdet, vec![10, 19, 20], vec![20], vec![10], vec![19]);
        do_round(&mut cdet, vec![21], vec![21], vec![], vec![]);
        do_round(&mut cdet, vec![], vec![22], vec![], vec![]);
        do_round(
            &mut cdet,
            vec![23, 24, 25, 26],
            vec![25, 27],
            vec![23, 26],
            vec![24],
        );
        do_round(&mut cdet, vec![23, 26, 28], vec![26, 28], vec![23], vec![]);

        // 10 rounds of just {23}
        for _ in 0..10 {
            do_round(&mut cdet, vec![23], vec![], vec![23], vec![]);
        }

        do_round(&mut cdet, vec![23, 29], vec![29], vec![23], vec![]);
        do_round(&mut cdet, vec![30, 31], vec![31], vec![30], vec![]);
        do_round(&mut cdet, vec![30], vec![30], vec![], vec![]);
        do_round(&mut cdet, vec![], vec![], vec![], vec![]);
    }
}
