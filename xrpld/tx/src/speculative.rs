//! Speculative execution scaffold for parallel transaction processing.
//!
//! Gated behind the `speculative-exec` feature.

use protocol::AccountID;
use std::collections::HashSet;

/// Tracks which account keys a transaction reads and writes.
#[derive(Debug, Default, Clone)]
pub struct ReadWriteSet {
    pub reads: HashSet<AccountID>,
    pub writes: HashSet<AccountID>,
}

/// Returns true if two read/write sets conflict (write-write or read-write overlap).
pub fn conflicts(a: &ReadWriteSet, b: &ReadWriteSet) -> bool {
    !a.writes.is_disjoint(&b.writes)
        || !a.writes.is_disjoint(&b.reads)
        || !a.reads.is_disjoint(&b.writes)
}

/// Partition transactions into groups of non-conflicting transactions.
///
/// Uses a greedy algorithm: for each transaction, try to place it in an
/// existing group where it conflicts with nothing; otherwise start a new group.
pub fn partition_independent<T>(txs: &[T]) -> Vec<Vec<&T>>
where
    T: AsReadWriteSet,
{
    let sets: Vec<ReadWriteSet> = txs.iter().map(|t| t.read_write_set()).collect();
    let mut groups: Vec<(Vec<&T>, ReadWriteSet)> = Vec::new();

    for (i, tx) in txs.iter().enumerate() {
        let mut placed = false;
        for (group_txs, group_set) in &mut groups {
            if !conflicts(&sets[i], group_set) {
                group_set.reads.extend(sets[i].reads.iter().cloned());
                group_set.writes.extend(sets[i].writes.iter().cloned());
                group_txs.push(tx);
                placed = true;
                break;
            }
        }
        if !placed {
            groups.push((vec![tx], sets[i].clone()));
        }
    }

    groups.into_iter().map(|(txs, _)| txs).collect()
}

/// Trait for types that can produce a [`ReadWriteSet`].
pub trait AsReadWriteSet {
    fn read_write_set(&self) -> ReadWriteSet;
}
