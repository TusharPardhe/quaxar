//! `PrunedStore`: the lifecycle wrapper the application drives in place of the
//! rotating node store when `[node_db] type = fjall`.
//!
//! It owns a key-value backend and an [`IndexWriter`], serializing every index
//! mutation behind one lock so the resurrection-race rules hold (design R1).
//! The application keeps the SHAMap knowledge: on each validated ledger it
//! diffs the trees and hands this store the resulting new/dead/owned hash sets.
//! This type does the rest: store node bytes, claim, prune on a schedule, and
//! expose metrics. It is engine-agnostic, so it is unit-tested on MemoryBackend
//! and runs on fjall unchanged.

use crate::backends::kv::{Keyspace, KvBatch};
use crate::pruned::index::{ClaimDelta, IndexWriter};
use crate::{Backend, DecodedBlob, EncodedBlob, NodeObject, Status};
use basics::base_uint::Uint256;
use std::sync::{Arc, Mutex};

/// How aggressively the store prunes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneMode {
    /// Compute what would be deleted and log it, but delete nothing. Used to
    /// validate the index against reachability before enabling deletes.
    DryRun,
    /// Delete dead nodes as the window advances.
    On,
}

/// Operator-facing configuration for the pruned store.
#[derive(Debug, Clone, Copy)]
pub struct PrunedConfig {
    /// Keep at least this many validated ledgers.
    pub online_delete: u32,
    /// Advisory prune cap (`can_delete`); `u32::MAX` means no cap.
    pub can_delete: u32,
    pub prune_mode: PruneMode,
    /// Notebook rows processed per prune batch.
    pub prune_batch: usize,
}

impl Default for PrunedConfig {
    fn default() -> Self {
        Self {
            online_delete: 512,
            can_delete: u32::MAX,
            prune_mode: PruneMode::On,
            prune_batch: 10_000,
        }
    }
}

/// Point-in-time counters for RPC/CLI (`get_counts`, `server_info`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrunedMetrics {
    pub claimed_seq: Option<u32>,
    pub pruned_to: u32,
    pub unclaimed: usize,
    /// Oldest validated ledger still fully retained (the advertised floor).
    pub retained_floor: u32,
    pub last_dry_run_would_delete: u64,
    pub verify_last_ok: bool,
}

pub struct PrunedStore {
    backend: Arc<dyn Backend>,
    writer: Mutex<IndexWriter>,
    config: PrunedConfig,
    metrics: Mutex<PrunedMetrics>,
}

impl PrunedStore {
    /// Open the store on an already-open, key-value-capable backend.
    pub fn open(backend: Arc<dyn Backend>, config: PrunedConfig) -> Result<Self, String> {
        let writer = IndexWriter::open(Arc::clone(&backend))?;
        let mut metrics = PrunedMetrics {
            verify_last_ok: true,
            ..PrunedMetrics::default()
        };
        metrics.claimed_seq = writer.claimed_seq();
        metrics.pruned_to = writer.pruned_to();
        Ok(Self {
            backend,
            writer: Mutex::new(writer),
            config,
            metrics: Mutex::new(metrics),
        })
    }

    /// Serve a node by hash from the `nodes` keyspace (peer and RPC reads).
    pub fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        match self.backend.kv_get(Keyspace::Nodes, hash.as_slice()) {
            Ok(Some(value)) => {
                let decoded = DecodedBlob::new(hash.data(), &value);
                if decoded.was_ok() {
                    (Some(decoded.create_object()), Status::Ok)
                } else {
                    (None, Status::DataCorrupt)
                }
            }
            Ok(None) => (None, Status::NotFound),
            Err(_) => (None, Status::Unknown),
        }
    }

    /// Store a node's bytes in the `nodes` keyspace and record it as not yet
    /// claimed into a validated ledger (so prune cannot delete it before the
    /// claim decides its fate; rule R2/R5).
    pub fn store(&self, object: &Arc<NodeObject>, current_seq: u32) -> Result<(), String> {
        let encoded = EncodedBlob::new(object);
        // Skip the node entirely if it is already present: re-storing a node
        // the index already tracks (e.g. a shared state node flushed again by a
        // later ledger) must not re-pin it in UNCLAIMED, or it would never be
        // pruned. A new node is written and recorded as not-yet-claimed.
        if self
            .backend
            .kv_get(Keyspace::Nodes, object.hash().as_slice())?
            .is_some()
        {
            return Ok(());
        }
        let mut batch = KvBatch::new();
        batch.put(
            Keyspace::Nodes,
            object.hash().as_slice().to_vec(),
            encoded.get_data().to_vec(),
        );
        self.backend.kv_write_batch(&batch)?;
        self.writer
            .lock()
            .expect("pruned store writer mutex")
            .note_stored(*object.hash(), current_seq);
        Ok(())
    }

    /// Claim a validated ledger from its node delta (computed by the caller via
    /// SHAMap diff). Updates the claimed-seq metric.
    pub fn claim(&self, delta: &ClaimDelta) -> Result<(), String> {
        let mut writer = self.writer.lock().expect("pruned store writer mutex");
        writer.claim(delta)?;
        let mut metrics = self.metrics.lock().expect("pruned store metrics mutex");
        metrics.claimed_seq = writer.claimed_seq();
        metrics.unclaimed = writer.unclaimed_len();
        Ok(())
    }

    /// Advance pruning for the current validated sequence. In `DryRun` mode it
    /// records how many nodes it would delete without deleting; in `On` mode it
    /// prunes to the window floor and runs the orphan sweep. Returns the number
    /// of nodes actually deleted (always 0 in dry-run), so the caller can
    /// invalidate caches only when the on-disk node set changed.
    pub fn maintain(&self, validated_seq: u32) -> Result<usize, String> {
        let k = self.prune_target(validated_seq);
        let mut writer = self.writer.lock().expect("pruned store writer mutex");
        let pruned = match self.config.prune_mode {
            PruneMode::DryRun => {
                let would = count_deletable(&self.backend, writer.pruned_to(), k)?;
                let mut metrics = self.metrics.lock().expect("metrics mutex");
                metrics.last_dry_run_would_delete = would;
                0
            }
            PruneMode::On => {
                let pruned = writer.prune(k, self.config.prune_batch)?;
                writer.orphan_sweep(validated_seq, self.config.online_delete)?;
                pruned
            }
        };
        let mut metrics = self.metrics.lock().expect("metrics mutex");
        metrics.pruned_to = writer.pruned_to();
        metrics.unclaimed = writer.unclaimed_len();
        metrics.retained_floor = k.saturating_add(1);
        Ok(pruned)
    }

    /// Oldest ledger still dropped: `min(validated - online_delete, can_delete)`.
    fn prune_target(&self, validated_seq: u32) -> u32 {
        let window_k = validated_seq.saturating_sub(self.config.online_delete);
        window_k.min(self.config.can_delete)
    }

    /// Verify every required node is present. The caller supplies the required
    /// set (from walking retained trees). Records the result in the metrics and
    /// returns the report so the caller can halt pruning on a miss.
    pub fn verify(
        &self,
        required: &std::collections::BTreeSet<Uint256>,
        sample_rate: usize,
    ) -> Result<crate::pruned::reconcile::VerifyReport, String> {
        let report =
            crate::pruned::reconcile::verify_present(self.backend.as_ref(), required, sample_rate)?;
        self.metrics.lock().expect("metrics mutex").verify_last_ok = report.is_ok();
        Ok(report)
    }

    pub fn metrics(&self) -> PrunedMetrics {
        *self.metrics.lock().expect("metrics mutex")
    }

    /// Adopt a freshly snapshot-imported store as the anchor ledger
    /// (design Case 7). The snapshot loader has already written the retained
    /// tree into `nodes` and verified both SHAMap roots. `required` is the
    /// exact set of hashes reachable from those roots (the snapshot's node
    /// set). This:
    ///
    ///   1. reconciles `nodes` down to `required`, reclaiming any leftovers a
    ///      prior sync left behind (sync stores ~9 M nodes with no index), and
    ///   2. stamps `anchor_seq`/`claimed_seq`/`claimed_state_root` so the next
    ///      validated ledger diffs against the snapshot's state root.
    ///
    /// It is only valid on a store with no prior claim.
    pub fn adopt_snapshot(
        &self,
        required: &std::collections::BTreeSet<Uint256>,
        anchor_seq: u32,
        state_root: Uint256,
    ) -> Result<usize, String> {
        let mut writer = self.writer.lock().expect("pruned store writer mutex");
        let swept = crate::pruned::reconcile::reconcile(
            self.backend.as_ref(),
            required,
            self.config.prune_batch,
        )?;
        writer.adopt_anchor(anchor_seq, state_root)?;
        let mut metrics = self.metrics.lock().expect("metrics mutex");
        metrics.claimed_seq = writer.claimed_seq();
        metrics.unclaimed = writer.unclaimed_len();
        Ok(swept)
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }
}

/// Count how many nodes a prune over `(pruned_to, k]` would delete, without
/// mutating. Mirrors the prune decision: STATE rows need count 0; OWNED rows
/// always count. Used by dry-run mode.
fn count_deletable(backend: &Arc<dyn Backend>, pruned_to: u32, k: u32) -> Result<u64, String> {
    use crate::backends::kv::{notebook_key, notebook_scan_end};
    if k <= pruned_to {
        return Ok(0);
    }
    let start = notebook_key(pruned_to.saturating_add(1), &Uint256::from_array([0; 32]));
    let end = notebook_scan_end(k);
    let mut would = 0u64;
    backend.kv_range(Keyspace::Notebook, &start, &end, &mut |key, value| {
        if key.len() == 4 + 32
            && let Some(hash) = Uint256::from_slice(&key[4..])
        {
            let kind = value.first().copied();
            let deletable = match kind {
                Some(1) => true, // Owned
                Some(0) => backend
                    .kv_get(Keyspace::Counts, hash.as_slice())
                    .ok()
                    .flatten()
                    .map(|b| b == 0u32.to_le_bytes())
                    .unwrap_or(false),
                _ => false,
            };
            if deletable {
                would += 1;
            }
        }
        true
    })?;
    Ok(would)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Factory, MemoryFactory, NodeObjectType, NullJournal};
    use basics::basic_config::Section;
    use std::sync::atomic::{AtomicU32, Ordering};

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
        section.set("path", format!("pruned-store-test/{n}"));
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

    fn node(hash: Uint256) -> Arc<NodeObject> {
        NodeObject::create_object(NodeObjectType::AccountNode, vec![9, 9, 9], hash)
    }

    #[test]
    fn store_then_fetch_round_trips() {
        let store = PrunedStore::open(open_memory(), PrunedConfig::default()).expect("open");
        let object = node(hid(1));
        store.store(&object, 1).expect("store");
        let (fetched, status) = store.fetch(object.hash());
        assert_eq!(status, Status::Ok);
        assert_eq!(fetched.expect("present").data(), object.data());
    }

    #[test]
    fn claim_and_prune_on_mode_drops_dead_nodes() {
        let config = PrunedConfig {
            online_delete: 2,
            prune_mode: PruneMode::On,
            ..PrunedConfig::default()
        };
        let store = PrunedStore::open(open_memory(), config).expect("open");
        // Three ledgers; node from ledger 1 dies at 2 and should prune once the
        // window (2) advances past it.
        for seq in 1..=4u32 {
            let n = node(hid(100 + seq as u64));
            store.store(&n, seq).expect("store");
            let dead = if seq == 1 {
                vec![]
            } else {
                vec![hid(100 + seq as u64 - 1)]
            };
            store
                .claim(&ClaimDelta {
                    seq,
                    state_root: hid(seq as u64),
                    new_state: vec![hid(100 + seq as u64)],
                    dead_state: dead,
                    owned: vec![],
                })
                .expect("claim");
            store.maintain(seq).expect("maintain");
        }
        // Window keeps seqs 3,4; node 101 (live only in ledger 1) is gone.
        assert_eq!(store.fetch(&hid(101)).1, Status::NotFound);
        // Node 104 (latest) is retained.
        assert_eq!(store.fetch(&hid(104)).1, Status::Ok);
        let metrics = store.metrics();
        assert_eq!(metrics.claimed_seq, Some(4));
    }

    #[test]
    fn dry_run_counts_without_deleting() {
        let config = PrunedConfig {
            online_delete: 1,
            prune_mode: PruneMode::DryRun,
            ..PrunedConfig::default()
        };
        let store = PrunedStore::open(open_memory(), config).expect("open");
        for seq in 1..=3u32 {
            let n = node(hid(200 + seq as u64));
            store.store(&n, seq).expect("store");
            let dead = if seq == 1 {
                vec![]
            } else {
                vec![hid(200 + seq as u64 - 1)]
            };
            store
                .claim(&ClaimDelta {
                    seq,
                    state_root: hid(seq as u64),
                    new_state: vec![hid(200 + seq as u64)],
                    dead_state: dead,
                    owned: vec![],
                })
                .expect("claim");
            store.maintain(seq).expect("maintain");
        }
        // Dry-run deletes nothing: every stored node is still present.
        assert_eq!(store.fetch(&hid(201)).1, Status::Ok);
        assert_eq!(store.fetch(&hid(202)).1, Status::Ok);
        // But it reported that it would have deleted the dead nodes.
        assert!(store.metrics().last_dry_run_would_delete >= 1);
    }

    #[test]
    fn adopt_snapshot_reconciles_leftovers_and_anchors() {
        use std::collections::BTreeSet;
        let backend = open_memory();
        // Simulate a sync/import that wrote nodes into `nodes` with no index:
        // two belong to the snapshot's tree, one is a leftover orphan.
        let keep_a = node(hid(10));
        let keep_b = node(hid(11));
        let leftover = node(hid(12));
        let mut raw = crate::backends::kv::KvBatch::new();
        for n in [&keep_a, &keep_b, &leftover] {
            raw.put(
                Keyspace::Nodes,
                n.hash().as_slice().to_vec(),
                EncodedBlob::new(n).get_data().to_vec(),
            );
        }
        backend.kv_write_batch(&raw).expect("seed nodes");

        let store = PrunedStore::open(Arc::clone(&backend), PrunedConfig::default()).expect("open");
        let required: BTreeSet<Uint256> = [hid(10), hid(11)].into_iter().collect();
        let swept = store
            .adopt_snapshot(&required, 500, hid(99))
            .expect("adopt");

        // The leftover is reclaimed; the snapshot's nodes remain servable.
        assert_eq!(swept, 1, "only the leftover is swept");
        assert_eq!(store.fetch(&hid(10)).1, Status::Ok);
        assert_eq!(store.fetch(&hid(11)).1, Status::Ok);
        assert_eq!(store.fetch(&hid(12)).1, Status::NotFound);
        // The anchor is the claimed sequence; the next claim diffs against it.
        assert_eq!(store.metrics().claimed_seq, Some(500));

        // A second adopt is refused once the store is anchored.
        assert!(store.adopt_snapshot(&required, 501, hid(98)).is_err());
    }

    #[test]
    fn verify_tracks_last_ok() {
        let store = PrunedStore::open(open_memory(), PrunedConfig::default()).expect("open");
        let present = node(hid(1));
        store.store(&present, 1).expect("store");
        let required: std::collections::BTreeSet<Uint256> = [hid(1), hid(2)].into_iter().collect();
        let report = store.verify(&required, 1).expect("verify");
        assert!(!report.is_ok(), "hid(2) is missing");
        assert!(!store.metrics().verify_last_ok);
    }
}
