//! Engine-agnostic index core for the pruned node store.
//!
//! [`IndexWriter`] is the single serialization point for every index mutation.
//! It turns the stream of validated ledgers into the stale-node index plus
//! reference counts described in `docs/design/2026-10-07-fjall-pruned-nodestore.md`,
//! and prunes nodes no retained ledger needs. It works over any
//! [`Backend`](crate::Backend) with `supports_kv()`, so the same logic is
//! tested on both MemoryBackend and the fjall backend.
//!
//! Semantics (see the design doc for the full case list):
//! - `claim(L)` records the nodes a validated ledger adds and retires relative
//!   to the previous claimed state, as one atomic batch. New nodes get a
//!   reference count; retired nodes get a notebook death record at `L`.
//! - `prune(K)` deletes nodes whose death record is at or below `K`, unless a
//!   later claim brought them back (count > 0) or they are pinned in
//!   `UNCLAIMED`/`DEAD_PENDING` (the resurrection-race rules R2/R5).
//! - `orphan_sweep` retires nodes stored but never claimed into a ledger.

use crate::Backend;
use crate::backends::kv::{Keyspace, KvBatch, PersistMode, notebook_key, notebook_scan_end};
use basics::base_uint::Uint256;
use std::collections::HashMap;
use std::sync::Arc;

/// Notebook record kind, stored as the single value byte of a notebook row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookKind {
    /// A state-tree node: deletable only when its reference count is zero.
    State = 0,
    /// A node owned by exactly one ledger (transaction tree or header):
    /// deletable unconditionally once its death sequence ages out.
    Owned = 1,
}

impl NotebookKind {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(NotebookKind::State),
            1 => Some(NotebookKind::Owned),
            _ => None,
        }
    }
}

/// Named META cursors. Values are fixed-width little-endian or raw hashes.
mod meta_key {
    pub const SCHEMA: &[u8] = b"schema";
    pub const CLAIMED_SEQ: &[u8] = b"claimed_seq";
    pub const CLAIMED_STATE_ROOT: &[u8] = b"claimed_state_root";
    pub const PRUNED_TO: &[u8] = b"pruned_to";
    pub const ANCHOR_SEQ: &[u8] = b"anchor_seq";
}

/// On-disk schema version for the pruned index.
pub const SCHEMA_VERSION: u32 = 1;

/// One validated ledger's node delta, computed by the caller via SHAMap diff
/// (`visit_differences`) against the previously claimed state root. The index
/// core is engine- and SHAMap-agnostic: it takes hashes directly.
#[derive(Debug, Clone, Default)]
pub struct ClaimDelta {
    pub seq: u32,
    pub state_root: Uint256,
    /// State nodes present in this ledger but not the previous claimed state.
    pub new_state: Vec<Uint256>,
    /// State nodes present in the previous claimed state but not this one.
    pub dead_state: Vec<Uint256>,
    /// Nodes owned solely by this ledger (tx-tree nodes and the header). They
    /// die when the next ledger arrives, so their death record is at `seq + 1`.
    pub owned: Vec<Uint256>,
}

/// The single-writer index over a key-value backend.
pub struct IndexWriter {
    backend: Arc<dyn Backend>,
    /// Nodes stored but not yet claimed into a validated ledger, with the
    /// sequence first seen. Prune never deletes these (rule R2).
    unclaimed: HashMap<Uint256, u32>,
    /// Mirror of the count==0 rows: nodes dead but not yet pruned, mapped to
    /// the sequence of their outstanding death record. Lets reuse pin them
    /// (rule R5) and lets a resurrection delete the exact stale record, both
    /// without a disk scan.
    dead_pending: HashMap<Uint256, u32>,
    claimed_seq: Option<u32>,
    pruned_to: u32,
}

impl IndexWriter {
    /// Open or initialize the index on `backend`, which must be KV-capable and
    /// already open. Reads back the META cursors so a restart resumes cleanly.
    pub fn open(backend: Arc<dyn Backend>) -> Result<Self, String> {
        if !backend.supports_kv() {
            return Err("IndexWriter requires a key-value backend".to_owned());
        }
        let schema = read_u32(backend.as_ref(), meta_key::SCHEMA)?;
        match schema {
            None => {
                // Fresh store: stamp the schema version.
                let mut batch = KvBatch::new();
                batch.put(
                    Keyspace::Meta,
                    meta_key::SCHEMA,
                    SCHEMA_VERSION.to_le_bytes().to_vec(),
                );
                backend.kv_write_batch(&batch)?;
            }
            Some(SCHEMA_VERSION) => {}
            Some(other) => {
                return Err(format!(
                    "unsupported pruned index schema {other}, expected {SCHEMA_VERSION}"
                ));
            }
        }
        let claimed_seq = read_u32(backend.as_ref(), meta_key::CLAIMED_SEQ)?;
        let pruned_to = read_u32(backend.as_ref(), meta_key::PRUNED_TO)?.unwrap_or(0);
        Ok(Self {
            backend,
            unclaimed: HashMap::new(),
            dead_pending: HashMap::new(),
            claimed_seq,
            pruned_to,
        })
    }

    pub fn claimed_seq(&self) -> Option<u32> {
        self.claimed_seq
    }

    pub fn pruned_to(&self) -> u32 {
        self.pruned_to
    }

    pub fn unclaimed_len(&self) -> usize {
        self.unclaimed.len()
    }

    /// Record that `hash` was stored (via flush or acquisition) but is not yet
    /// owned by a validated ledger. Rule R5: if it is dead-pending, pin it so
    /// prune keeps it until the next claim decides its fate.
    pub fn note_stored(&mut self, hash: Uint256, current_seq: u32) {
        self.unclaimed.entry(hash).or_insert(current_seq);
    }

    /// Claim a validated ledger. Applies all count/notebook/meta mutations in
    /// one atomic batch, then persists. Idempotent: a seq at or below the last
    /// claim is ignored (safe restart replay).
    pub fn claim(&mut self, delta: &ClaimDelta) -> Result<(), String> {
        // Idempotent after a restart replay: ignore a seq already claimed.
        if matches!(self.claimed_seq, Some(prev) if delta.seq <= prev) {
            return Ok(());
        }
        let mut batch = KvBatch::new();
        // New state nodes: a node absent from counts is implicitly live-once
        // (count 1). A node that was dead-pending (explicit 0) or shared
        // (explicit >=2) has a row; bump it. `encode_count` drops the row when
        // the result is exactly 1, restoring the implicit representation.
        for hash in &delta.new_state {
            let current = self.count_get(hash)?.unwrap_or(0);
            let updated = current.saturating_add(1);
            self.encode_count(&mut batch, hash, updated);
            // Resurrection: a node coming back from count 0 has an outstanding
            // death record. Delete it so a later prune does not act on a stale
            // death seq and remove a now-live node.
            if let Some(dead_seq) = self.dead_pending.remove(hash) {
                batch.delete(Keyspace::Notebook, notebook_key(dead_seq, hash));
            }
            self.unclaimed.remove(hash);
        }
        // Dead state nodes: decrement; a drop to zero writes a notebook record.
        for hash in &delta.dead_state {
            let current = self.count_get(hash)?.unwrap_or(1);
            let updated = current.saturating_sub(1);
            self.encode_count(&mut batch, hash, updated);
            if updated == 0 {
                batch.put(
                    Keyspace::Notebook,
                    notebook_key(delta.seq, hash),
                    vec![NotebookKind::State as u8],
                );
                self.dead_pending.insert(*hash, delta.seq);
            }
        }
        // Owned nodes die when the next ledger arrives.
        for hash in &delta.owned {
            batch.put(
                Keyspace::Notebook,
                notebook_key(delta.seq.saturating_add(1), hash),
                vec![NotebookKind::Owned as u8],
            );
            self.unclaimed.remove(hash);
        }
        // Advance cursors in the same atomic batch.
        batch.put(
            Keyspace::Meta,
            meta_key::CLAIMED_SEQ,
            delta.seq.to_le_bytes().to_vec(),
        );
        batch.put(
            Keyspace::Meta,
            meta_key::CLAIMED_STATE_ROOT,
            delta.state_root.as_slice().to_vec(),
        );
        if self.claimed_seq.is_none() {
            batch.put(
                Keyspace::Meta,
                meta_key::ANCHOR_SEQ,
                delta.seq.to_le_bytes().to_vec(),
            );
        }
        self.backend.kv_write_batch(&batch)?;
        self.backend.kv_persist(PersistMode::SyncAll)?;
        self.claimed_seq = Some(delta.seq);
        Ok(())
    }

    /// Adopt a snapshot-loaded store as the anchor ledger (design Case 7).
    ///
    /// A snapshot import writes the whole retained tree into `nodes` with no
    /// index history. This is the `claim(A)` with `P = ∅` case: every node is
    /// implicitly live-once (count 1, so no explicit count rows), the dead set
    /// is empty, and `A` becomes both the claimed sequence and the anchor. It
    /// is only valid on a store with no prior claim; a store that already has
    /// a claimed sequence returns an error rather than rewinding its cursors.
    pub fn adopt_anchor(&mut self, seq: u32, state_root: Uint256) -> Result<(), String> {
        if let Some(prev) = self.claimed_seq {
            return Err(format!(
                "cannot adopt snapshot anchor {seq}: store already claimed {prev}"
            ));
        }
        let mut batch = KvBatch::new();
        batch.put(
            Keyspace::Meta,
            meta_key::CLAIMED_SEQ,
            seq.to_le_bytes().to_vec(),
        );
        batch.put(
            Keyspace::Meta,
            meta_key::CLAIMED_STATE_ROOT,
            state_root.as_slice().to_vec(),
        );
        batch.put(
            Keyspace::Meta,
            meta_key::ANCHOR_SEQ,
            seq.to_le_bytes().to_vec(),
        );
        self.backend.kv_write_batch(&batch)?;
        self.backend.kv_persist(PersistMode::SyncAll)?;
        self.claimed_seq = Some(seq);
        Ok(())
    }

    /// Prune every notebook record at or below `k`. A STATE record deletes its
    /// node only if the reference count is zero and it is not pinned; an OWNED
    /// record deletes unconditionally (unless pinned). Processes in bounded
    /// chunks so a crash resumes from `pruned_to`.
    pub fn prune(&mut self, k: u32, chunk: usize) -> Result<usize, String> {
        if k <= self.pruned_to {
            return Ok(0);
        }
        let chunk = chunk.max(1);
        let mut total = 0usize;
        loop {
            let start = notebook_key(
                self.pruned_to.saturating_add(1),
                &Uint256::from_array([0; 32]),
            );
            let end = notebook_scan_end(k);
            // Collect one chunk of records under the scan.
            let mut rows: Vec<(u32, Uint256, NotebookKind)> = Vec::new();
            self.backend
                .kv_range(Keyspace::Notebook, &start, &end, &mut |key, value| {
                    if let Some((seq, hash)) = decode_notebook_key(key)
                        && let Some(kind) = value.first().and_then(|b| NotebookKind::from_byte(*b))
                    {
                        rows.push((seq, hash, kind));
                    }
                    rows.len() < chunk
                })?;
            if rows.is_empty() {
                break;
            }
            let mut batch = KvBatch::new();
            let mut highest = self.pruned_to;
            for (seq, hash, kind) in &rows {
                highest = highest.max(*seq);
                let pinned = self.unclaimed.contains_key(hash);
                let deletable = !pinned
                    && match kind {
                        NotebookKind::Owned => true,
                        NotebookKind::State => self.count_get(hash)?.unwrap_or(0) == 0,
                    };
                if deletable {
                    batch.delete(Keyspace::Nodes, hash.as_slice().to_vec());
                    batch.delete(Keyspace::Counts, hash.as_slice().to_vec());
                    total += 1;
                }
                batch.delete(Keyspace::Notebook, notebook_key(*seq, hash));
                self.dead_pending.remove(hash);
            }
            // Only advance pruned_to to `k` once the final chunk is done; while
            // chunks remain, advance to the highest fully-scanned seq minus one
            // is unsafe (same seq may span chunks), so advance to k only when
            // the scan returned fewer than a full chunk.
            let final_chunk = rows.len() < chunk;
            let advance_to = if final_chunk {
                k
            } else {
                highest.saturating_sub(1)
            };
            batch.put(
                Keyspace::Meta,
                meta_key::PRUNED_TO,
                advance_to.to_le_bytes().to_vec(),
            );
            self.backend.kv_write_batch(&batch)?;
            self.pruned_to = advance_to;
            if final_chunk {
                break;
            }
        }
        self.backend.kv_persist(PersistMode::SyncAll)?;
        Ok(total)
    }

    /// Retire nodes stored but never claimed older than the retention window.
    /// They become count-0 state records at `current_seq` and prune normally.
    pub fn orphan_sweep(&mut self, current_seq: u32, online_delete: u32) -> Result<usize, String> {
        let cutoff = current_seq.saturating_sub(online_delete);
        let orphans: Vec<Uint256> = self
            .unclaimed
            .iter()
            .filter(|(_, first_seen)| **first_seen < cutoff)
            .map(|(hash, _)| *hash)
            .collect();
        if orphans.is_empty() {
            return Ok(0);
        }
        let mut batch = KvBatch::new();
        for hash in &orphans {
            self.encode_count(&mut batch, hash, 0);
            batch.put(
                Keyspace::Notebook,
                notebook_key(current_seq, hash),
                vec![NotebookKind::State as u8],
            );
            self.unclaimed.remove(hash);
            self.dead_pending.insert(*hash, current_seq);
        }
        self.backend.kv_write_batch(&batch)?;
        Ok(orphans.len())
    }

    // --- encoding helpers -------------------------------------------------

    fn count_get(&self, hash: &Uint256) -> Result<Option<u32>, String> {
        match self.backend.kv_get(Keyspace::Counts, hash.as_slice())? {
            Some(bytes) if bytes.len() == 4 => {
                Ok(Some(u32::from_le_bytes(bytes.try_into().expect("4 bytes"))))
            }
            Some(_) => Err("corrupt count row (expected 4 bytes)".to_owned()),
            None => Ok(None),
        }
    }

    /// Encode a count into the batch. A count of exactly 1 is represented by
    /// the absence of a row, so storing 1 deletes any explicit row.
    fn encode_count(&self, batch: &mut KvBatch, hash: &Uint256, count: u32) {
        if count == 1 {
            batch.delete(Keyspace::Counts, hash.as_slice().to_vec());
        } else {
            batch.put(
                Keyspace::Counts,
                hash.as_slice().to_vec(),
                count.to_le_bytes().to_vec(),
            );
        }
    }
}

fn read_u32(backend: &dyn Backend, key: &[u8]) -> Result<Option<u32>, String> {
    match backend.kv_get(Keyspace::Meta, key)? {
        Some(bytes) if bytes.len() == 4 => {
            Ok(Some(u32::from_le_bytes(bytes.try_into().expect("4 bytes"))))
        }
        Some(_) => Err(format!(
            "corrupt meta value for {}",
            String::from_utf8_lossy(key)
        )),
        None => Ok(None),
    }
}

fn decode_notebook_key(key: &[u8]) -> Option<(u32, Uint256)> {
    if key.len() != 4 + 32 {
        return None;
    }
    let seq = u32::from_be_bytes(key[..4].try_into().ok()?);
    let hash = Uint256::from_slice(&key[4..])?;
    Some((seq, hash))
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod index_tests;
