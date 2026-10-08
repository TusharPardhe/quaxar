//! Key-value storage primitives shared by the pruned node store.
//!
//! The historical [`Backend`](crate::Backend) trait is a single hash -> blob
//! table. The pruned store needs several logically separate tables (nodes,
//! a notebook of death records, reference counts, and metadata cursors) plus
//! per-key deletes and atomic cross-table writes, which an append-only hash store cannot express.
//! These types model that capability without disturbing the hash -> blob API,
//! so existing backends keep working through default trait methods.

use basics::base_uint::Uint256;

/// A logically separate table within one backend ("column family" / fjall
/// keyspace). The ordering of the enum fixes the on-disk/in-memory partition
/// identity, so variants must not be reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Keyspace {
    /// Hash -> NodeObject bytes. The same data the legacy backend serves.
    Nodes,
    /// `seq(BE u32) || hash` -> death-record kind byte. Sorted by ledger seq
    /// so pruning is a single forward range scan.
    Notebook,
    /// Hash -> reference count (`u32` LE). Absent means a count of one.
    Counts,
    /// Small named cursors: schema version, claimed/pruned sequences, anchor.
    Meta,
}

impl Keyspace {
    /// Stable identifier used by backends that name their partitions.
    pub const fn name(self) -> &'static str {
        match self {
            Keyspace::Nodes => "nodes",
            Keyspace::Notebook => "notebook",
            Keyspace::Counts => "counts",
            Keyspace::Meta => "meta",
        }
    }

    /// Every keyspace, in a fixed order, for backend initialization.
    pub const ALL: [Keyspace; 4] = [
        Keyspace::Nodes,
        Keyspace::Notebook,
        Keyspace::Counts,
        Keyspace::Meta,
    ];
}

/// One mutation inside a [`KvBatch`]. Keys and values are raw bytes so the
/// index layer owns all encoding (big-endian sequences, count widths, kinds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvOp {
    Put {
        keyspace: Keyspace,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        keyspace: Keyspace,
        key: Vec<u8>,
    },
}

impl KvOp {
    pub fn keyspace(&self) -> Keyspace {
        match self {
            KvOp::Put { keyspace, .. } | KvOp::Delete { keyspace, .. } => *keyspace,
        }
    }
}

/// An ordered set of mutations applied atomically across keyspaces: either all
/// of them become visible or none do, even across a crash. This is the unit the
/// pruned store commits per validated ledger and per prune chunk.
#[derive(Debug, Clone, Default)]
pub struct KvBatch {
    ops: Vec<KvOp>,
}

impl KvBatch {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ops: Vec::with_capacity(capacity),
        }
    }

    pub fn put(
        &mut self,
        keyspace: Keyspace,
        key: impl Into<Vec<u8>>,
        value: impl Into<Vec<u8>>,
    ) -> &mut Self {
        self.ops.push(KvOp::Put {
            keyspace,
            key: key.into(),
            value: value.into(),
        });
        self
    }

    pub fn delete(&mut self, keyspace: Keyspace, key: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(KvOp::Delete {
            keyspace,
            key: key.into(),
        });
        self
    }

    pub fn ops(&self) -> &[KvOp] {
        &self.ops
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// Durability requested when persisting a backend's journal. Mirrors the
/// coarse choices every engine offers, so the index layer can ask for a hard
/// flush after a validated ledger and skip it on best-effort paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistMode {
    /// Flush to OS buffers only. Survives a process crash, not a power loss.
    Buffer,
    /// Flush all the way to the storage medium (fsync).
    SyncAll,
}

/// Node-store key for a notebook death record (`seq || hash`), encoded so that
/// a lexicographic scan visits records in ascending ledger order.
pub fn notebook_key(seq: u32, hash: &Uint256) -> Vec<u8> {
    let mut key = Vec::with_capacity(4 + 32);
    key.extend_from_slice(&seq.to_be_bytes());
    key.extend_from_slice(hash.as_slice());
    key
}

/// Inclusive upper bound for scanning the notebook up to and including `seq`.
/// Returns the first key strictly greater than any record at `seq`.
pub fn notebook_scan_end(seq: u32) -> Vec<u8> {
    // seq+1 with a zero hash is the smallest key beyond every (seq, *) record.
    // At u32::MAX there is no larger seq, so an all-ones key bounds the scan.
    match seq.checked_add(1) {
        Some(next) => next.to_be_bytes().to_vec(),
        None => vec![0xFF; 4 + 32],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyspace_names_are_stable_and_distinct() {
        let names: Vec<&str> = Keyspace::ALL.iter().map(|ks| ks.name()).collect();
        assert_eq!(names, ["nodes", "notebook", "counts", "meta"]);
    }

    #[test]
    fn notebook_keys_sort_by_sequence_then_hash() {
        let low = notebook_key(1, &Uint256::from_array([0xFF; 32]));
        let high = notebook_key(2, &Uint256::from_array([0x00; 32]));
        // A later sequence outranks any hash at an earlier sequence.
        assert!(high > low);
    }

    #[test]
    fn notebook_scan_end_excludes_next_sequence() {
        let last_at_seq = notebook_key(5, &Uint256::from_array([0xFF; 32]));
        let end = notebook_scan_end(5);
        assert!(end > last_at_seq);
        let first_at_next = notebook_key(6, &Uint256::from_array([0x00; 32]));
        assert!(end <= first_at_next);
    }

    #[test]
    fn batch_records_ops_in_order() {
        let mut batch = KvBatch::new();
        batch
            .put(Keyspace::Nodes, vec![1], vec![2])
            .delete(Keyspace::Counts, vec![3]);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.ops()[0].keyspace(), Keyspace::Nodes);
        assert_eq!(batch.ops()[1].keyspace(), Keyspace::Counts);
    }
}
