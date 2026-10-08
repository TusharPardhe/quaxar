//! Verification and reconciliation for the pruned node store.
//!
//! The index writer keeps the `nodes` keyspace in step with the retained
//! window incrementally. These two safety nets catch any drift:
//!
//! - [`verify_present`] confirms that every node the retained window requires
//!   is physically present. A missing live node is a correctness failure the
//!   caller must treat as fatal (stop pruning, refetch from peers).
//! - [`reconcile`] is the mark-sweep repair: given the exact required set, it
//!   deletes `nodes` entries that are not required (reclaiming leaked
//!   orphans) and clears their stale count rows. It never deletes a required
//!   node, so it is always safe to run.
//!
//! Both take the required node-hash set directly, so this module stays
//! engine- and SHAMap-agnostic. The integration layer computes that set by
//! walking the retained ledgers' trees.

use crate::Backend;
use crate::backends::kv::{Keyspace, KvBatch};
use basics::base_uint::Uint256;
use std::collections::BTreeSet;

/// Outcome of a verification pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Required nodes that are missing from the `nodes` keyspace. Non-empty is
    /// a correctness failure.
    pub missing: Vec<Uint256>,
    /// Number of required nodes checked.
    pub checked: usize,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Confirm every hash in `required` is present in the `nodes` keyspace.
///
/// `sample_rate` of 1 checks every node; `n` checks roughly every `n`th node
/// (cheaper periodic verification). A sample that finds a missing node still
/// reports it, but only a full pass (`sample_rate == 1`) proves completeness.
pub fn verify_present(
    backend: &dyn Backend,
    required: &BTreeSet<Uint256>,
    sample_rate: usize,
) -> Result<VerifyReport, String> {
    let step = sample_rate.max(1);
    let mut report = VerifyReport::default();
    for (index, hash) in required.iter().enumerate() {
        if index % step != 0 {
            continue;
        }
        report.checked += 1;
        if backend.kv_get(Keyspace::Nodes, hash.as_slice())?.is_none() {
            report.missing.push(*hash);
        }
    }
    Ok(report)
}

/// Delete every `nodes` entry not in `required`, plus its count row. Returns
/// the number of nodes swept. This repairs leaks (orphans the incremental path
/// missed) and is safe because it only removes nodes the window does not need.
///
/// `chunk` bounds the batch size so a large sweep commits incrementally.
pub fn reconcile(
    backend: &dyn Backend,
    required: &BTreeSet<Uint256>,
    chunk: usize,
) -> Result<usize, String> {
    let chunk = chunk.max(1);
    // Collect every stored node hash first (a snapshot), then delete the ones
    // not required. Collecting up front avoids mutating during the scan.
    let mut stored: Vec<Uint256> = Vec::new();
    backend.kv_range(Keyspace::Nodes, &[], &[0xFF; 64], &mut |key, _| {
        if let Some(hash) = Uint256::from_slice(key) {
            stored.push(hash);
        }
        true
    })?;

    let mut swept = 0usize;
    let mut batch = KvBatch::new();
    for hash in stored {
        if required.contains(&hash) {
            continue;
        }
        batch.delete(Keyspace::Nodes, hash.as_slice().to_vec());
        batch.delete(Keyspace::Counts, hash.as_slice().to_vec());
        swept += 1;
        if batch.len() >= chunk * 2 {
            backend.kv_write_batch(&batch)?;
            batch = KvBatch::new();
        }
    }
    if !batch.is_empty() {
        backend.kv_write_batch(&batch)?;
    }
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::kv::KvBatch;
    use crate::{Backend, Factory, MemoryFactory, NodeObject, NullJournal};
    use basics::basic_config::Section;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    fn hid(n: u64) -> Uint256 {
        let mut b = [0u8; 32];
        b[24..].copy_from_slice(&n.to_be_bytes());
        Uint256::from_array(b)
    }

    fn open_memory() -> Arc<dyn Backend> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut section = Section::new("node_db");
        section.set("type", "Memory");
        section.set("path", format!("reconcile-test/{n}"));
        let backend = MemoryFactory::new()
            .create_instance(
                NodeObject::KEY_BYTES,
                &section,
                0,
                Arc::new(crate::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("memory backend");
        let backend: Arc<dyn Backend> = Arc::from(backend);
        backend.open(true).expect("open");
        backend
    }

    fn put_nodes(backend: &dyn Backend, hashes: &[Uint256]) {
        let mut batch = KvBatch::new();
        for hash in hashes {
            batch.put(Keyspace::Nodes, hash.as_slice().to_vec(), vec![0xAB]);
        }
        backend.kv_write_batch(&batch).expect("put nodes");
    }

    #[test]
    fn verify_reports_missing_required_nodes() {
        let backend = open_memory();
        put_nodes(backend.as_ref(), &[hid(1), hid(2)]);
        let required: BTreeSet<Uint256> = [hid(1), hid(2), hid(3)].into_iter().collect();
        let report = verify_present(backend.as_ref(), &required, 1).expect("verify");
        assert_eq!(report.checked, 3);
        assert_eq!(report.missing, vec![hid(3)]);
        assert!(!report.is_ok());
    }

    #[test]
    fn verify_passes_when_all_present() {
        let backend = open_memory();
        put_nodes(backend.as_ref(), &[hid(1), hid(2), hid(3)]);
        let required: BTreeSet<Uint256> = [hid(1), hid(2), hid(3)].into_iter().collect();
        let report = verify_present(backend.as_ref(), &required, 1).expect("verify");
        assert!(report.is_ok());
        assert_eq!(report.checked, 3);
    }

    #[test]
    fn reconcile_sweeps_unrequired_nodes_only() {
        let backend = open_memory();
        put_nodes(backend.as_ref(), &[hid(1), hid(2), hid(3), hid(4)]);
        // Also give hid(2) and hid(4) explicit count rows to prove they are
        // cleared alongside the node.
        let mut counts = KvBatch::new();
        counts.put(
            Keyspace::Counts,
            hid(4).as_slice().to_vec(),
            0u32.to_le_bytes().to_vec(),
        );
        backend.kv_write_batch(&counts).expect("counts");

        let required: BTreeSet<Uint256> = [hid(1), hid(2)].into_iter().collect();
        let swept = reconcile(backend.as_ref(), &required, 8).expect("reconcile");
        assert_eq!(swept, 2, "hid(3) and hid(4) are not required");

        // Required nodes remain; unrequired are gone, with their count rows.
        assert!(
            backend
                .kv_get(Keyspace::Nodes, hid(1).as_slice())
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .kv_get(Keyspace::Nodes, hid(2).as_slice())
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .kv_get(Keyspace::Nodes, hid(3).as_slice())
                .unwrap()
                .is_none()
        );
        assert!(
            backend
                .kv_get(Keyspace::Nodes, hid(4).as_slice())
                .unwrap()
                .is_none()
        );
        assert!(
            backend
                .kv_get(Keyspace::Counts, hid(4).as_slice())
                .unwrap()
                .is_none()
        );

        // Verify now passes against the required set.
        let report = verify_present(backend.as_ref(), &required, 1).expect("verify");
        assert!(report.is_ok());
    }

    #[test]
    fn reconcile_on_empty_required_sweeps_all() {
        let backend = open_memory();
        put_nodes(backend.as_ref(), &[hid(1), hid(2), hid(3)]);
        let swept = reconcile(backend.as_ref(), &BTreeSet::new(), 2).expect("reconcile");
        assert_eq!(swept, 3);
    }
}
