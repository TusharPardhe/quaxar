//! Automatic snapshot scheduling.
//!
//! The `SnapshotScheduler` tracks ledger close events and triggers a snapshot
//! export every N ledgers. The export runs on a background thread to avoid
//! blocking the main consensus path.
//!
//! # Snapshot Isolation
//!
//! The export calls `Backend::for_each` which iterates all stored nodes.
//! Consistency guarantees depend on the backend implementation:
//!
//! - **NuDB**: The data file is append-only. Iteration reads sequentially through
//!   the data file. Concurrent writes append new records but do not modify existing
//!   ones. The iterator sees a consistent view of all records that existed at the
//!   time iteration started, plus potentially some records appended during iteration.
//!   This is acceptable: extra nodes do not corrupt the snapshot (they are simply
//!   additional data the loader writes to the target backend).
//!
//! - **RocksDB**: The iterator internally holds an LSM snapshot at creation time.
//!   Concurrent writes are invisible to the iterator. This provides strict
//!   point-in-time consistency automatically.
//!
//! - **MemoryBackend** (tests only): Uses a `HashMap` behind a `Mutex`. The
//!   `for_each` implementation locks and clones the map, providing a consistent
//!   snapshot of all entries at call time.
//!
//! In all production backends, the export produces a superset of nodes that existed
//! at the target ledger sequence. The snapshot header records the `account_hash`
//! (SHAMap root) so that after loading, the consumer can reconstruct the SHAMap and
//! verify the root matches — any extraneous nodes are harmless (they exist in the
//! store but are not reachable from the verified root).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;

use super::manifest::{SNAPSHOT_VERSION, SnapshotManifest};
use super::writer::export_snapshot;
use crate::Backend;

/// Configuration for automatic snapshot production.
#[derive(Debug, Clone)]
pub struct SnapshotSchedulerConfig {
    /// Export a snapshot every `interval` ledgers. 0 means disabled.
    pub interval: u32,
    /// Directory where snapshot files are written.
    pub output_dir: PathBuf,
}

impl Default for SnapshotSchedulerConfig {
    fn default() -> Self {
        Self {
            interval: 0,
            output_dir: PathBuf::from("."),
        }
    }
}

/// Tracks ledger closes and triggers snapshot exports on a background thread.
pub struct SnapshotScheduler {
    config: SnapshotSchedulerConfig,
    last_snapshot_seq: AtomicU32,
    export_in_progress: Arc<AtomicBool>,
}

impl SnapshotScheduler {
    pub fn new(config: SnapshotSchedulerConfig) -> Self {
        Self {
            config,
            last_snapshot_seq: AtomicU32::new(0),
            export_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns true if automatic snapshots are enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.interval > 0
    }

    /// Called after every validated ledger close. If this ledger triggers a
    /// snapshot, spawns a background export and returns `true`.
    ///
    /// The export iterates the backend without holding any application-level lock.
    /// See module-level documentation for snapshot isolation guarantees per backend.
    pub fn on_ledger_accepted(
        &self,
        ledger_seq: u32,
        ledger_hash: [u8; 32],
        account_hash: [u8; 32],
        backend: Arc<dyn Backend>,
    ) -> bool {
        if !self.is_enabled() {
            return false;
        }

        if !ledger_seq.is_multiple_of(self.config.interval) {
            return false;
        }

        // Don't start a new export if one is already running
        if self.export_in_progress.swap(true, Ordering::AcqRel) {
            tracing::warn!(
                target: "snapshot",
                ledger_seq,
                "Skipping automatic snapshot: previous export still in progress"
            );
            return false;
        }

        self.last_snapshot_seq.store(ledger_seq, Ordering::Release);

        let output_dir = self.config.output_dir.clone();
        let in_progress = Arc::clone(&self.export_in_progress);

        let spawn_result = thread::Builder::new()
            .name(format!("snapshot-export-{ledger_seq}"))
            .spawn(move || {
                let hash_hex: String = ledger_hash[..4]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let filename = format!("snapshot-{ledger_seq}-{hash_hex}.xrpls");
                let output_path = output_dir.join(&filename);

                let manifest = SnapshotManifest {
                    version: SNAPSHOT_VERSION,
                    ledger_seq,
                    ledger_hash,
                    account_hash,
                    tx_hash: [0u8; 32],
                    parent_hash: [0u8; 32],
                    drops: 0,
                    close_time: 0,
                    parent_close_time: 0,
                    close_time_res: 10,
                    close_flags: 0,
                    chunks: Vec::new(),
                };

                match export_snapshot(backend.as_ref(), &manifest, &output_path) {
                    Ok(()) => {
                        tracing::info!(
                            target: "snapshot",
                            ledger_seq,
                            path = %output_path.display(),
                            "Automatic snapshot export completed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "snapshot",
                            error = %e,
                            ledger_seq,
                            "Automatic snapshot export failed"
                        );
                    }
                }

                in_progress.store(false, Ordering::Release);
            });

        if let Err(e) = spawn_result {
            tracing::error!(
                target: "snapshot",
                error = %e,
                "Failed to spawn snapshot export thread"
            );
            self.export_in_progress.store(false, Ordering::Release);
        }

        true
    }

    /// Returns the last ledger sequence for which a snapshot was triggered.
    pub fn last_snapshot_seq(&self) -> u32 {
        self.last_snapshot_seq.load(Ordering::Acquire)
    }

    /// Returns true if an export is currently running.
    pub fn is_exporting(&self) -> bool {
        self.export_in_progress.load(Ordering::Acquire)
    }
}
