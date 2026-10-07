//! Drives the pruned node store from the stream of validated ledgers.
//!
//! When `[node_db] type = fjall`, the SHAMap store component holds a
//! `PrunedDriver` instead of running the rotation worker. On each validated
//! ledger the driver computes the node delta against the previously claimed
//! ledger ([`compute_claim_delta`]), claims it into the pruned index, and
//! advances pruning to the retention-window floor. It keeps the last claimed
//! ledger resident so the next diff has a `have` tree to compare against;
//! holding one extra ledger is cheap relative to the window the store already
//! retains.

use crate::shamap::pruned_claim::compute_claim_delta;
use ledger::Ledger;
use nodestore::{PruneMode, PrunedConfig, PrunedMetrics, PrunedStore};
use std::sync::{Arc, Mutex};

/// Owns the pruned store and the bookkeeping the claim path needs across
/// ledgers. Cheap to clone-share via `Arc`; all mutable state is behind one
/// mutex so claims stay serialized (design R1).
pub struct PrunedDriver {
    store: Arc<PrunedStore>,
    /// The last ledger claimed into the index. The next claim diffs against
    /// its state tree. `None` until the first (anchor) claim.
    last_claimed: Mutex<Option<Arc<Ledger>>>,
}

impl PrunedDriver {
    pub fn new(store: Arc<PrunedStore>) -> Self {
        Self {
            store,
            last_claimed: Mutex::new(None),
        }
    }

    /// Open a driver over an already-open, key-value-capable backend.
    pub fn open(
        backend: Arc<dyn nodestore::Backend>,
        config: PrunedConfig,
    ) -> Result<Self, String> {
        let store = PrunedStore::open(backend, config)?;
        Ok(Self::new(Arc::new(store)))
    }

    /// Claim a validated ledger and advance pruning. Called from the SHAMap
    /// store component's `on_ledger_closed` for the fjall path.
    ///
    /// Errors are returned rather than panicking: the caller logs them and
    /// keeps serving, since a failed claim leaves the index unchanged (the
    /// batch is atomic) and the next ledger re-diffs across the gap.
    pub fn on_validated_ledger(&self, ledger: Arc<Ledger>) -> Result<(), String> {
        let seq = ledger.header().seq;
        let state_root = *ledger.header().account_hash.as_uint256();

        let mut last = self
            .last_claimed
            .lock()
            .expect("pruned driver last-claimed mutex");
        let delta = {
            let prev_state = last.as_ref().map(|l| l.state_map());
            compute_claim_delta(ledger.as_ref(), prev_state, state_root)
        };
        self.store.claim(&delta)?;
        self.store.maintain(seq)?;
        *last = Some(ledger);
        Ok(())
    }

    pub fn metrics(&self) -> PrunedMetrics {
        self.store.metrics()
    }

    pub fn store(&self) -> &Arc<PrunedStore> {
        &self.store
    }
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
    PrunedConfig {
        online_delete,
        can_delete,
        prune_mode,
        prune_batch,
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
        let config = pruned_config_from_section(&section, 512, u32::MAX);
        assert_eq!(config.prune_mode, PruneMode::DryRun);
        assert_eq!(config.prune_batch, 500);
        assert_eq!(config.online_delete, 512);
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
