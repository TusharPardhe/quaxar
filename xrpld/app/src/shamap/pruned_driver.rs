//! Drives the pruned node store from the stream of validated ledgers.
//!
//! When `[node_db] type = fjall`, the SHAMap store component holds a
//! `PrunedDriver`. Validated ledgers are handed to [`PrunedDriver::submit`],
//! which only records the newest ledger and wakes a dedicated worker thread,
//! so the validated-ledger publish path never does pruning I/O. The worker
//! computes the node delta against the previously claimed ledger
//! ([`compute_claim_delta`]), claims it, advances pruning to the window floor,
//! and periodically verifies. Intermediate ledgers can be skipped under load:
//! the next claim diffs across the gap against the last claimed state, which
//! yields a larger but still exact delta.

use crate::shamap::pruned_claim::{ClaimNodeFetcher, compute_claim_delta};
use basics::sha_map_hash::SHAMapHash;
use ledger::Ledger;
use nodestore::{
    Backend, DecodedBlob, Keyspace, PruneMode, PrunedConfig, PrunedMetrics, PrunedStore,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// Owns the pruned store and the bookkeeping the claim path needs across
/// ledgers. All claim/prune work is serialized on one worker (design R1).
pub struct PrunedDriver {
    store: Arc<PrunedStore>,
    /// Reads backed SHAMap nodes from this store when a validated ledger's
    /// subtree is not resident, so claim deltas are complete.
    fetch: Arc<ClaimNodeFetcher>,
    /// The last ledger claimed into the index. The next claim diffs against
    /// its state tree. `None` until the first (anchor) claim.
    last_claimed: Mutex<Option<Arc<Ledger>>>,
    /// Invoked after a prune that actually deleted nodes, to invalidate the
    /// FullBelowCache (design Case 16).
    on_prune: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// Invoked after every maintain pass with the retained floor
    /// (`pruned_to + 1`), to lower the advertised `complete_ledgers` (Case 18).
    on_floor_advanced: Mutex<Option<Box<dyn Fn(u32) + Send + Sync>>>,
    /// When the last sampled verify ran.
    last_verify: Mutex<Option<Instant>>,
    /// Live advisory prune cap (`can_delete`), updated by the RPC.
    can_delete: Arc<AtomicU32>,
    /// Newest validated ledger waiting for the worker (latest wins).
    pending: Mutex<Option<Arc<Ledger>>>,
    wake: Condvar,
    stopping: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl PrunedDriver {
    pub fn new(store: Arc<PrunedStore>) -> Self {
        let backend = Arc::clone(store.backend());
        Self {
            fetch: store_fetcher(backend),
            store,
            last_claimed: Mutex::new(None),
            on_prune: Mutex::new(None),
            on_floor_advanced: Mutex::new(None),
            last_verify: Mutex::new(None),
            can_delete: Arc::new(AtomicU32::new(u32::MAX)),
            pending: Mutex::new(None),
            wake: Condvar::new(),
            stopping: AtomicBool::new(false),
            worker: Mutex::new(None),
        }
    }

    /// Open a driver over an already-open, key-value-capable backend.
    pub fn open(backend: Arc<dyn Backend>, config: PrunedConfig) -> Result<Self, String> {
        let store = PrunedStore::open(backend, config)?;
        Ok(Self::new(Arc::new(store)))
    }

    /// Register a callback run after any prune that deleted nodes.
    pub fn set_on_prune(&self, callback: Box<dyn Fn() + Send + Sync>) {
        *self.on_prune.lock().expect("pruned driver on_prune mutex") = Some(callback);
    }

    /// Register a callback run after every maintain pass with the retained
    /// floor (design Case 18).
    pub fn set_on_floor_advanced(&self, callback: Box<dyn Fn(u32) + Send + Sync>) {
        *self
            .on_floor_advanced
            .lock()
            .expect("pruned driver on_floor_advanced mutex") = Some(callback);
    }

    /// Shared handle to the live advisory prune cap. The SHAMap store
    /// component stores the effective `can_delete` here on every update.
    pub fn can_delete_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.can_delete)
    }

    /// Start the worker thread. Idempotent.
    pub fn spawn_worker(self: &Arc<Self>) {
        let mut worker = self.worker.lock().expect("pruned driver worker mutex");
        if worker.is_some() {
            return;
        }
        self.stopping.store(false, Ordering::Release);
        let driver = Arc::clone(self);
        *worker = Some(
            std::thread::Builder::new()
                .name("pruned-store".to_owned())
                .spawn(move || driver.run_worker())
                .expect("spawn pruned-store worker"),
        );
    }

    /// Stop the worker after it finishes the ledger in hand.
    pub fn stop_worker(&self) {
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_all();
        if let Some(handle) = self
            .worker
            .lock()
            .expect("pruned driver worker mutex")
            .take()
        {
            let _ = handle.join();
        }
    }

    /// Hand a validated ledger to the worker. Never blocks on store I/O: it
    /// replaces any ledger the worker has not picked up yet.
    pub fn submit(&self, ledger: Arc<Ledger>) {
        *self.pending.lock().expect("pruned driver pending mutex") = Some(ledger);
        self.wake.notify_one();
    }

    fn run_worker(&self) {
        // Pruning is background maintenance: never compete with consensus.
        #[cfg(target_os = "linux")]
        // SAFETY: setpriority on the calling thread (who = 0) only changes this
        // thread's nice value.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        }
        loop {
            let ledger = {
                let mut pending = self.pending.lock().expect("pruned driver pending mutex");
                loop {
                    if self.stopping.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(ledger) = pending.take() {
                        break ledger;
                    }
                    pending = self
                        .wake
                        .wait(pending)
                        .expect("pruned driver pending condvar");
                }
            };
            let seq = ledger.header().seq;
            if let Err(error) = self.on_validated_ledger(ledger) {
                tracing::warn!(
                    target: "nodestore",
                    seq,
                    %error,
                    "pruned store claim/maintain failed; will re-diff on the next ledger"
                );
            }
        }
    }

    /// Claim a validated ledger, advance pruning and periodically verify. The
    /// worker calls this; tests call it directly for determinism.
    ///
    /// A delta that cannot be computed completely (a backed node neither
    /// resident nor in the store) aborts before the index is touched, so a
    /// partial claim can never strand live nodes for the orphan sweep.
    pub fn on_validated_ledger(&self, ledger: Arc<Ledger>) -> Result<(), String> {
        let seq = ledger.header().seq;
        let state_root = *ledger.header().account_hash.as_uint256();

        let mut last = self
            .last_claimed
            .lock()
            .expect("pruned driver last-claimed mutex");
        let claimed_seq = self.store.metrics().claimed_seq;
        if claimed_seq.is_some_and(|claimed| seq <= claimed) {
            return Ok(());
        }
        let Some(_) = claimed_seq else {
            // First validated ledger on an unclaimed store: anchor it without
            // walking its (multi-million node) state tree. Every node present
            // is implicitly live-once; later claims diff against this root.
            self.store.anchor(seq, state_root)?;
            *last = Some(Arc::clone(&ledger));
            tracing::info!(target: "nodestore", seq, "pruned store anchored");
            return Ok(());
        };
        // Diff against the resident previous claimed ledger, or after a restart
        // against the persisted claimed root read back from the store.
        let prev_root = match last.as_ref() {
            Some(prev) => prev.state_map().root(),
            None => {
                let hash = self
                    .store
                    .claimed_state_root()?
                    .ok_or_else(|| "claimed store has no claimed state root".to_owned())?;
                (self.fetch)(SHAMapHash::new(hash))
                    .ok_or_else(|| format!("claimed state root {hash} is not in the store"))?
            }
        };
        let delta =
            compute_claim_delta(ledger.as_ref(), &prev_root, state_root, self.fetch.as_ref())?;
        self.store.claim(&delta)?;
        *last = Some(Arc::clone(&ledger));
        drop(last);

        let pruned = self
            .store
            .maintain(seq, self.can_delete.load(Ordering::Acquire))?;
        if pruned > 0
            && let Some(callback) = self
                .on_prune
                .lock()
                .expect("pruned driver on_prune mutex")
                .as_ref()
        {
            callback();
        }
        if let Some(callback) = self
            .on_floor_advanced
            .lock()
            .expect("pruned driver on_floor_advanced mutex")
            .as_ref()
        {
            callback(self.store.metrics().retained_floor);
        }
        self.maybe_verify(ledger.as_ref())
    }

    /// Run a verify of `ledger`'s reachable nodes plus the window-wide count
    /// check if `verify_interval` has elapsed. A miss halts pruning
    /// permanently (design Case 11): deleting more nodes on top of a known
    /// inconsistency could make it unrecoverable.
    fn maybe_verify(&self, ledger: &Ledger) -> Result<(), String> {
        let interval = self.store.verify_interval_secs();
        if interval == 0 {
            return Ok(());
        }
        {
            let last = self.last_verify.lock().expect("pruned driver verify mutex");
            if let Some(at) = *last
                && at.elapsed().as_secs() < interval
            {
                return Ok(());
            }
        }
        let mut required = std::collections::BTreeSet::new();
        for tree in [ledger.state_map(), ledger.tx_map()] {
            let mut fetch = |hash| (self.fetch)(hash);
            tree.visit_nodes(&mut fetch, &mut |node| {
                required.insert(*node.get_hash().as_uint256());
                true
            })
            .map_err(|error| format!("verify traversal failed: {error:?}"))?;
        }
        let report = self.store.verify(&required, 1)?;
        let window = self.store.verify_window()?;
        *self.last_verify.lock().expect("pruned driver verify mutex") = Some(Instant::now());
        if !report.is_ok() || !window.is_ok() {
            self.store.halt_pruning();
            let message = format!(
                "pruned store verify failed at seq {}: {} of {} latest-ledger nodes missing, \
                 {} of {} windowed count nodes missing; pruning halted",
                ledger.header().seq,
                report.missing.len(),
                report.checked,
                window.missing.len(),
                window.checked
            );
            tracing::error!(target: "nodestore", "{message}");
            return Err(message);
        }
        Ok(())
    }

    pub fn metrics(&self) -> PrunedMetrics {
        self.store.metrics()
    }

    pub fn store(&self) -> &Arc<PrunedStore> {
        &self.store
    }
}

impl Drop for PrunedDriver {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}

/// Node-store reader for backed SHAMap traversal during claims and verify.
fn store_fetcher(backend: Arc<dyn Backend>) -> Arc<ClaimNodeFetcher> {
    Arc::new(move |hash: SHAMapHash| {
        let bytes = backend
            .kv_get(Keyspace::Nodes, hash.as_uint256().as_slice())
            .ok()
            .flatten()?;
        let decoded = DecodedBlob::new(hash.as_uint256().data(), &bytes);
        if !decoded.was_ok() {
            return None;
        }
        let object = decoded.create_object();
        shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(object.data(), hash).ok()
    })
}

/// Read the pruned-store configuration from a `[node_db]` section. Unknown or
/// missing fields fall back to defaults; `prune_mode` accepts `on`/`dry_run`.
pub fn pruned_config_from_section(
    node_db: &basics::basic_config::Section,
    online_delete: u32,
    can_delete: u32,
) -> PrunedConfig {
    let prune_mode = node_db
        .get::<String>("prune_mode")
        .ok()
        .flatten()
        .map(|value| {
            if value.eq_ignore_ascii_case("dry_run") {
                PruneMode::DryRun
            } else {
                PruneMode::On
            }
        })
        .unwrap_or(PruneMode::On);
    let prune_batch = node_db
        .get::<usize>("prune_batch")
        .ok()
        .flatten()
        .filter(|&n| n > 0)
        .unwrap_or(10_000);
    let verify_interval_secs = node_db
        .get::<u64>("verify_interval")
        .ok()
        .flatten()
        .unwrap_or(3600);
    PrunedConfig {
        online_delete,
        can_delete,
        prune_mode,
        prune_batch,
        verify_interval_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::basic_config::Section;

    #[test]
    fn config_parses_prune_mode_and_batch() {
        let mut section = Section::new("node_db");
        section.set("prune_mode", "dry_run");
        section.set("prune_batch", "500");
        section.set("verify_interval", "900");
        let config = pruned_config_from_section(&section, 512, u32::MAX);
        assert_eq!(config.prune_mode, PruneMode::DryRun);
        assert_eq!(config.prune_batch, 500);
        assert_eq!(config.online_delete, 512);
        assert_eq!(config.verify_interval_secs, 900);
    }

    #[test]
    fn config_defaults_when_absent() {
        let section = Section::new("node_db");
        let config = pruned_config_from_section(&section, 256, 100);
        assert_eq!(config.prune_mode, PruneMode::On);
        assert_eq!(config.prune_batch, 10_000);
        assert_eq!(config.online_delete, 256);
        assert_eq!(config.can_delete, 100);
    }

    #[test]
    fn config_rejects_zero_batch() {
        let mut section = Section::new("node_db");
        section.set("prune_batch", "0");
        let config = pruned_config_from_section(&section, 512, u32::MAX);
        assert_eq!(
            config.prune_batch, 10_000,
            "zero batch falls back to default"
        );
    }
}
