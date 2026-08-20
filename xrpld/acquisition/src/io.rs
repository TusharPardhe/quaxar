//! NodeStore read/write/fence surface.
//!
//! Storage owns physical I/O; the coordinator owns the pending-read graph,
//! write intent, in-flight accounting, retry policy, and the final durability
//! decision. Every request carries an [`OperationRef`] and every completion is
//! a typed event the coordinator validates before it may mutate a session.

use basics::sha_map_hash::SHAMapHash;
use bytes::Bytes;

use crate::id::StoreGeneration;
use crate::identity::{OperationKind, OperationRef};

/// Read admission priority. Consensus/recovery work must not be starved by
/// history work; capacity reservation is enforced at the broker port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReadPriority {
    /// Consensus, validation, or recovery demand.
    Consensus,
    /// History fill / sequential catchup.
    History,
}

/// A brokered NodeStore read. The broker owns physical read admission,
/// coalescing by `(key, ledger sequence, store generation)`, and settles
/// exactly one completion per logical subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    operation: OperationRef,
    key: SHAMapHash,
    ledger_sequence: u32,
    store_generation: StoreGeneration,
    priority: ReadPriority,
}

impl ReadRequest {
    /// Builds a read request. `operation.kind()` must be [`OperationKind::Read`].
    pub fn new(
        operation: OperationRef,
        key: SHAMapHash,
        ledger_sequence: u32,
        store_generation: StoreGeneration,
        priority: ReadPriority,
    ) -> Self {
        debug_assert_eq!(operation.kind(), OperationKind::Read);
        Self {
            operation,
            key,
            ledger_sequence,
            store_generation,
            priority,
        }
    }

    /// The exact operation identity of this read.
    pub const fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The node key to read.
    pub const fn key(&self) -> SHAMapHash {
        self.key
    }

    /// The ledger sequence scope of the read.
    pub const fn ledger_sequence(&self) -> u32 {
        self.ledger_sequence
    }

    /// The database generation scope of the read.
    pub const fn store_generation(&self) -> StoreGeneration {
        self.store_generation
    }

    /// The admission priority of the read.
    pub const fn priority(&self) -> ReadPriority {
        self.priority
    }
}

/// The outcome of one brokered read completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    /// The physical read settled. `node` is `None` when the key was not present
    /// in the store generation.
    Settled { node: Option<Bytes> },
    /// The session or operation is no longer live; the completion is stale and
    /// must not mutate a session.
    Stale,
    /// The read was explicitly cancelled.
    Cancelled,
}

/// A typed read completion reported by the broker. The coordinator validates
/// `operation` against the expected in-flight read before mutating state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadCompletion {
    operation: OperationRef,
    outcome: ReadOutcome,
}

impl ReadCompletion {
    /// Builds a read completion.
    pub const fn new(operation: OperationRef, outcome: ReadOutcome) -> Self {
        Self { operation, outcome }
    }

    /// The operation this completion reports.
    pub const fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The completion outcome.
    pub fn outcome(&self) -> &ReadOutcome {
        &self.outcome
    }
}

/// The NodeStore object classification for a persisted node. Preserved from
/// the traversal's store commands so the write adapter writes the exact object
/// type the legacy path wrote (rippled `InboundLedgerStore` store-object
/// parity). NuDB keys by hash only, so this does not affect read addressing,
/// but the object classification participates in the encoded record and in
/// post-store cache promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoredObjectKind {
    /// A ledger object (header/metadata).
    Ledger,
    /// A state tree node.
    AccountNode,
    /// A transaction tree node.
    TransactionNode,
    /// The traversal did not classify the node.
    Unknown,
}

/// One node to persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistNode {
    key: SHAMapHash,
    data: Bytes,
    object_kind: StoredObjectKind,
}

impl PersistNode {
    /// Builds a node to persist.
    pub const fn new(key: SHAMapHash, data: Bytes, object_kind: StoredObjectKind) -> Self {
        Self {
            key,
            data,
            object_kind,
        }
    }

    /// The node key.
    pub const fn key(&self) -> SHAMapHash {
        self.key
    }

    /// The node wire data.
    pub fn data(&self) -> &Bytes {
        &self.data
    }

    /// The NodeStore object classification of this node.
    pub const fn object_kind(&self) -> StoredObjectKind {
        self.object_kind
    }
}

/// A write batch submitted to the NodeStore write adapter. The write adapter
/// owns physical write submission only.
///
/// A batch also carries the `fence` operation: the durability barrier that the
/// adapter must run after the batch and report as one `DurabilityFenced`
/// event. Carrying both identities lets a single adapter submission produce the
/// write completion and the fence completion without a second coordinator
/// effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteBatch {
    operation: OperationRef,
    fence: OperationRef,
    store_generation: StoreGeneration,
    /// The verified ledger header sequence that owns every record in this batch.
    /// It is never inferred from a NodeStore key or an untrusted peer node.
    ledger_sequence: u32,
    nodes: Vec<PersistNode>,
}

impl WriteBatch {
    /// Builds a write batch. `operation.kind()` must be [`OperationKind::Write`]
    /// and `fence.kind()` must be [`OperationKind::DurabilityFence`].
    pub fn new(
        operation: OperationRef,
        fence: OperationRef,
        store_generation: StoreGeneration,
        ledger_sequence: u32,
        nodes: Vec<PersistNode>,
    ) -> Self {
        debug_assert_eq!(operation.kind(), OperationKind::Write);
        debug_assert_eq!(fence.kind(), OperationKind::DurabilityFence);
        Self {
            operation,
            fence,
            store_generation,
            ledger_sequence,
            nodes,
        }
    }

    /// The exact operation identity of this write.
    pub const fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The exact operation identity of the durability barrier to run after the
    /// batch. The adapter reports its completion as a `DurabilityFenced` event.
    pub const fn fence(&self) -> OperationRef {
        self.fence
    }

    /// The database generation this batch targets.
    pub const fn store_generation(&self) -> StoreGeneration {
        self.store_generation
    }

    /// The verified ledger header sequence associated with every node.
    pub const fn ledger_sequence(&self) -> u32 {
        self.ledger_sequence
    }

    /// The nodes to persist.
    pub fn nodes(&self) -> &[PersistNode] {
        &self.nodes
    }

    /// Total payload bytes in this batch.
    pub fn payload_bytes(&self) -> usize {
        self.nodes.iter().map(|n| n.data.len()).sum()
    }
}

/// The outcome of one write completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The batch was durably submitted.
    Accepted,
    /// The physical write failed.
    Failed,
    /// The session or operation is no longer live.
    Stale,
    /// The write was explicitly cancelled.
    Cancelled,
}

/// A typed write completion reported by the NodeStore write adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCompletion {
    operation: OperationRef,
    outcome: WriteOutcome,
}

impl WriteCompletion {
    /// Builds a write completion.
    pub const fn new(operation: OperationRef, outcome: WriteOutcome) -> Self {
        Self { operation, outcome }
    }

    /// The operation this completion reports.
    pub const fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The completion outcome.
    pub const fn outcome(&self) -> WriteOutcome {
        self.outcome
    }
}

/// The outcome of the durability fence (final persistence barrier).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityOutcome {
    /// The barrier passed; the ledger is durable and safe to hand off.
    Passed,
    /// The barrier failed; no normal adoptable ledger may be produced.
    Failed,
    /// The session or operation is no longer live.
    Stale,
}

/// A typed durability-fence completion. A passed fence is required before the
/// coordinator may issue a durable handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurabilityCompletion {
    operation: OperationRef,
    outcome: DurabilityOutcome,
}

impl DurabilityCompletion {
    /// Builds a durability completion.
    pub const fn new(operation: OperationRef, outcome: DurabilityOutcome) -> Self {
        Self { operation, outcome }
    }

    /// The operation this completion reports.
    pub const fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The completion outcome.
    pub const fn outcome(&self) -> DurabilityOutcome {
        self.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, OperationGeneration, OperationId, PlanEpoch, RunEpoch, SessionId};
    use crate::identity::SessionRef;
    use basics::base_uint::Uint256;

    fn session() -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    #[test]
    fn read_request_carries_full_identity() {
        let mut counter = IdCounter::new();
        let operation = OperationRef::new(
            session(),
            OperationKind::Read,
            counter.next_id(),
            counter.next_id(),
        );
        let request = ReadRequest::new(
            operation,
            SHAMapHash::new(Uint256::from(9)),
            42,
            StoreGeneration::new(1),
            ReadPriority::Consensus,
        );
        assert_eq!(request.operation(), operation);
        assert_eq!(request.key(), SHAMapHash::new(Uint256::from(9)));
        assert_eq!(request.ledger_sequence(), 42);
        assert_eq!(request.store_generation(), StoreGeneration::new(1));
        assert_eq!(request.priority(), ReadPriority::Consensus);
    }

    #[test]
    fn write_batch_reports_payload_bytes_and_fence() {
        let mut counter = IdCounter::new();
        let session = session();
        let operation = OperationRef::new(
            session,
            OperationKind::Write,
            counter.next_id(),
            counter.next_id(),
        );
        let fence = OperationRef::new(
            session,
            OperationKind::DurabilityFence,
            counter.next_id(),
            counter.next_id(),
        );
        let batch = WriteBatch::new(
            operation,
            fence,
            StoreGeneration::new(1),
            42,
            vec![
                PersistNode::new(
                    SHAMapHash::new(Uint256::from(1)),
                    Bytes::from_static(&[1, 2, 3]),
                    StoredObjectKind::AccountNode,
                ),
                PersistNode::new(
                    SHAMapHash::new(Uint256::from(2)),
                    Bytes::from_static(&[4, 5]),
                    StoredObjectKind::TransactionNode,
                ),
            ],
        );
        assert_eq!(batch.operation(), operation);
        assert_eq!(batch.fence(), fence);
        assert_eq!(batch.ledger_sequence(), 42);
        assert_eq!(batch.payload_bytes(), 5);
        assert_eq!(
            batch.nodes()[0].object_kind(),
            StoredObjectKind::AccountNode,
            "the object classification survives into the batch"
        );
    }

    #[test]
    fn operation_ids_never_invalid_on_dispatched_ops() {
        let mut counter = IdCounter::new();
        let operation = OperationRef::new(
            session(),
            OperationKind::DurabilityFence,
            counter.next_id(),
            counter.next_id(),
        );
        assert_ne!(operation.operation_id(), OperationId::INVALID);
        assert_ne!(operation.generation(), OperationGeneration::INVALID);
    }
}
