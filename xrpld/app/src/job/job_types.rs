//! Job type definitions and their static configuration (priority ordering,
//! concurrency limits). Ported from rippled's `Job.h`, `JobTypeInfo.h`, and
//! `JobTypes.h`.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

/// The category of work a [`crate::job::job_queue::Job`] performs.
///
/// Variant order defines job priority: earlier variants have *lower*
/// priority than later ones, matching the reference enum's documented
/// invariant ("the position in this enum indicates the job priority with
/// earlier jobs having lower priority than later jobs"). `JobQueue` picks
/// the highest-priority runnable job first (see
/// [`crate::job::job_queue::JobQueue`]'s scheduling doc comment).
///
/// `JtInvalid` (`-1` in the reference, used as a sentinel for "no job type
/// yet assigned") is intentionally not modeled as a variant here: Rust's
/// `Job` type only exists once fully constructed with a real `JobType`, so
/// there is no code path that needs an invalid placeholder value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum JobType {
    /// Make a fetch pack for a peer.
    JtPack,
    /// An old ledger has been accepted.
    JtPuboldledger,
    /// Placeholder for the priority of all `Jt*Client*` jobs.
    JtClient,
    /// A websocket subscription by a client.
    JtClientSubscribe,
    /// Subscription for fee change by a client.
    JtClientFeeChange,
    /// Subscription for consensus state change by a client.
    JtClientConsensus,
    /// Subscription for account history by a client.
    JtClientAcctHist,
    /// Client RPC request.
    JtClientRpc,
    /// Client websocket request.
    JtClientWebsocket,
    /// A websocket command from the client.
    JtRpc,
    /// Sweep for stale structures.
    JtSweep,
    /// A validation from an untrusted source.
    JtValidationUt,
    /// A validator's manifest.
    JtManifest,
    /// Update pathfinding requests.
    JtUpdatePf,
    /// A local transaction.
    JtTransactionL,
    /// Peer request for a ledger delta or a skip list.
    JtReplayReq,
    /// Peer request for ledger/txnset data.
    JtLedgerReq,
    /// A proposal from an untrusted source.
    JtProposalUt,
    /// A ledger replay task/subtask.
    JtReplayTask,
    /// A transaction received from the network.
    JtTransaction,
    /// Request missing transactions.
    JtMissingTxn,
    /// Reply with requested transactions.
    JtRequestedTxn,
    /// Apply batched transactions.
    JtBatch,
    /// Received data for a ledger we're acquiring.
    JtLedgerData,
    /// Advance validated/acquired ledgers.
    JtAdvance,
    /// Publish a fully-accepted ledger.
    JtPubledger,
    /// Fetch a proposed set.
    JtTxnData,
    /// Write-ahead logging.
    JtWal,
    /// A validation from a trusted source.
    JtValidationT,
    /// Write out hashed objects.
    JtWrite,
    /// Accept a consensus ledger.
    JtAccept,
    /// A proposal from a trusted source.
    JtProposalT,
    /// NetworkOPs cluster peer report.
    JtNetopCluster,
    /// NetworkOPs net timer processing.
    JtNetopTimer,
    /// An administrative operation.
    JtAdmin,

    // Special job types which are not dispatched by the job pool (limit 0).
    JtPeer,
    JtDisk,
    JtTxnProc,
    JtObSetup,
    JtPathFind,
    JtHoRead,
    JtHoWrite,
    /// Used just to measure time.
    JtGeneric,

    // Node store monitoring.
    JtNsSyncRead,
    JtNsAsyncRead,
    JtNsWrite,
}

impl JobType {
    /// All variants, in reference declaration (priority) order.
    pub const ALL: [JobType; 46] = [
        JobType::JtPack,
        JobType::JtPuboldledger,
        JobType::JtClient,
        JobType::JtClientSubscribe,
        JobType::JtClientFeeChange,
        JobType::JtClientConsensus,
        JobType::JtClientAcctHist,
        JobType::JtClientRpc,
        JobType::JtClientWebsocket,
        JobType::JtRpc,
        JobType::JtSweep,
        JobType::JtValidationUt,
        JobType::JtManifest,
        JobType::JtUpdatePf,
        JobType::JtTransactionL,
        JobType::JtReplayReq,
        JobType::JtLedgerReq,
        JobType::JtProposalUt,
        JobType::JtReplayTask,
        JobType::JtTransaction,
        JobType::JtMissingTxn,
        JobType::JtRequestedTxn,
        JobType::JtBatch,
        JobType::JtLedgerData,
        JobType::JtAdvance,
        JobType::JtPubledger,
        JobType::JtTxnData,
        JobType::JtWal,
        JobType::JtValidationT,
        JobType::JtWrite,
        JobType::JtAccept,
        JobType::JtProposalT,
        JobType::JtNetopCluster,
        JobType::JtNetopTimer,
        JobType::JtAdmin,
        JobType::JtPeer,
        JobType::JtDisk,
        JobType::JtTxnProc,
        JobType::JtObSetup,
        JobType::JtPathFind,
        JobType::JtHoRead,
        JobType::JtHoWrite,
        JobType::JtGeneric,
        JobType::JtNsSyncRead,
        JobType::JtNsAsyncRead,
        JobType::JtNsWrite,
    ];

    /// Static configuration for this job type. Matches
    /// `JobTypes::instance().get(jt)`.
    pub fn info(self) -> &'static JobTypeInfo {
        registry()
            .get(&self)
            .expect("every JobType variant has a registered JobTypeInfo")
    }

    /// Display name. Matches `JobTypes::name(jt)`.
    pub fn name(self) -> &'static str {
        &self.info().name
    }

    /// The concurrency limit for this job type. `usize::MAX` signals no
    /// limit (matches the reference's use of `INT_MAX` for "maxLimit").
    /// A limit of `0` marks this a "special" job type never dispatched
    /// through the queue's worker pool.
    pub fn limit(self) -> usize {
        self.info().limit
    }

    /// Whether this is a "special" job type (limit `0`), not dispatched
    /// via the job queue's worker pool. Matches `JobTypeInfo::special()`.
    pub fn is_special(self) -> bool {
        self.limit() == 0
    }
}

/// Holds the static, unchanging information about a job type. Matches
/// `JobTypeInfo`.
#[derive(Debug, Clone)]
pub struct JobTypeInfo {
    pub job_type: JobType,
    pub name: String,
    pub limit: usize,
    pub avg_latency: Duration,
    pub peak_latency: Duration,
}

fn registry() -> &'static BTreeMap<JobType, JobTypeInfo> {
    static REGISTRY: OnceLock<BTreeMap<JobType, JobTypeInfo>> = OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

/// Builds the static job type table. Matches `JobTypes::JobTypes()`'s
/// `add(...)` call list verbatim (name, limit, avg/peak latency).
fn build_registry() -> BTreeMap<JobType, JobTypeInfo> {
    const MAX: usize = usize::MAX;
    let entries: &[(JobType, &str, usize, u64, u64)] = &[
        (JobType::JtPack, "makeFetchPack", 1, 0, 0),
        (
            JobType::JtPuboldledger,
            "publishAcqLedger",
            2,
            10_000,
            15_000,
        ),
        (
            JobType::JtValidationUt,
            "untrustedValidation",
            MAX,
            2_000,
            5_000,
        ),
        (JobType::JtManifest, "manifest", MAX, 2_000, 5_000),
        (JobType::JtTransactionL, "localTransaction", MAX, 100, 500),
        (JobType::JtReplayReq, "ledgerReplayRequest", 10, 250, 1_000),
        (JobType::JtLedgerReq, "ledgerRequest", 3, 0, 0),
        (JobType::JtProposalUt, "untrustedProposal", MAX, 500, 1_250),
        (JobType::JtReplayTask, "ledgerReplayTask", MAX, 0, 0),
        (JobType::JtLedgerData, "ledgerData", 3, 0, 0),
        (JobType::JtClient, "clientCommand", MAX, 2_000, 5_000),
        (
            JobType::JtClientSubscribe,
            "clientSubscribe",
            MAX,
            2_000,
            5_000,
        ),
        (
            JobType::JtClientFeeChange,
            "clientFeeChange",
            MAX,
            2_000,
            5_000,
        ),
        (
            JobType::JtClientConsensus,
            "clientConsensus",
            MAX,
            2_000,
            5_000,
        ),
        (
            JobType::JtClientAcctHist,
            "clientAccountHistory",
            MAX,
            2_000,
            5_000,
        ),
        (JobType::JtClientRpc, "clientRPC", MAX, 2_000, 5_000),
        (
            JobType::JtClientWebsocket,
            "clientWebsocket",
            MAX,
            2_000,
            5_000,
        ),
        (JobType::JtRpc, "RPC", MAX, 0, 0),
        (JobType::JtUpdatePf, "updatePaths", 1, 0, 0),
        (JobType::JtTransaction, "transaction", MAX, 250, 1_000),
        (JobType::JtBatch, "batch", MAX, 250, 1_000),
        (JobType::JtAdvance, "advanceLedger", MAX, 0, 0),
        (JobType::JtPubledger, "publishNewLedger", MAX, 3_000, 4_500),
        (JobType::JtTxnData, "fetchTxnData", 5, 0, 0),
        (JobType::JtWal, "writeAhead", MAX, 1_000, 2_500),
        (JobType::JtValidationT, "trustedValidation", MAX, 500, 1_500),
        (JobType::JtWrite, "writeObjects", MAX, 1_750, 2_500),
        (JobType::JtAccept, "acceptLedger", MAX, 0, 0),
        (JobType::JtProposalT, "trustedProposal", MAX, 100, 500),
        (JobType::JtSweep, "sweep", 1, 0, 0),
        (JobType::JtNetopCluster, "clusterReport", 1, 9_999, 9_999),
        (JobType::JtNetopTimer, "heartbeat", 1, 999, 999),
        (JobType::JtAdmin, "administration", MAX, 0, 0),
        (JobType::JtMissingTxn, "handleHaveTransactions", 1_200, 0, 0),
        (JobType::JtRequestedTxn, "doTransactions", 1_200, 0, 0),
        (JobType::JtPeer, "peerCommand", 0, 200, 2_500),
        (JobType::JtDisk, "diskAccess", 0, 500, 1_000),
        (JobType::JtTxnProc, "processTransaction", 0, 0, 0),
        (JobType::JtObSetup, "orderBookSetup", 0, 0, 0),
        (JobType::JtPathFind, "pathFind", 0, 0, 0),
        (JobType::JtHoRead, "nodeRead", 0, 0, 0),
        (JobType::JtHoWrite, "nodeWrite", 0, 0, 0),
        (JobType::JtGeneric, "generic", 0, 0, 0),
        (JobType::JtNsSyncRead, "SyncReadNode", 0, 0, 0),
        (JobType::JtNsAsyncRead, "AsyncReadNode", 0, 0, 0),
        (JobType::JtNsWrite, "WriteNode", 0, 0, 0),
    ];

    entries
        .iter()
        .map(|&(job_type, name, limit, avg_ms, peak_ms)| {
            (
                job_type,
                JobTypeInfo {
                    job_type,
                    name: name.to_string(),
                    limit,
                    avg_latency: Duration::from_millis(avg_ms),
                    peak_latency: Duration::from_millis(peak_ms),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_type_has_registered_info() {
        for &jt in JobType::ALL.iter() {
            let info = jt.info();
            assert_eq!(info.job_type, jt);
        }
    }

    #[test]
    fn special_job_types_have_zero_limit() {
        for jt in [
            JobType::JtPeer,
            JobType::JtDisk,
            JobType::JtTxnProc,
            JobType::JtObSetup,
            JobType::JtPathFind,
            JobType::JtHoRead,
            JobType::JtHoWrite,
            JobType::JtGeneric,
            JobType::JtNsSyncRead,
            JobType::JtNsAsyncRead,
            JobType::JtNsWrite,
        ] {
            assert!(jt.is_special(), "{jt:?} should be special (limit 0)");
        }
    }

    #[test]
    fn non_special_job_types_have_positive_limit() {
        for jt in [
            JobType::JtPack,
            JobType::JtUpdatePf,
            JobType::JtSweep,
            JobType::JtNetopCluster,
            JobType::JtNetopTimer,
        ] {
            assert!(!jt.is_special());
            assert!(
                jt.limit() > 0 && jt.limit() < usize::MAX,
                "{jt:?} should have a finite positive limit"
            );
        }
    }

    #[test]
    fn unlimited_job_types_report_usize_max() {
        assert_eq!(JobType::JtClient.limit(), usize::MAX);
        assert_eq!(JobType::JtTransaction.limit(), usize::MAX);
    }

    #[test]
    fn priority_order_matches_reference_declaration_order() {
        // JtPack is the lowest priority; JtAdmin is the highest among
        // pool-dispatched types; special types sort after JtAdmin.
        assert!(JobType::JtPack < JobType::JtClient);
        assert!(JobType::JtClient < JobType::JtAdmin);
        assert!(JobType::JtAdmin < JobType::JtPeer);
        assert!(JobType::JtPeer < JobType::JtGeneric);
    }

    #[test]
    fn latency_values_match_reference_table_spot_checks() {
        assert_eq!(
            JobType::JtPuboldledger.info().avg_latency,
            Duration::from_secs(10)
        );
        assert_eq!(
            JobType::JtPuboldledger.info().peak_latency,
            Duration::from_secs(15)
        );
        assert_eq!(
            JobType::JtNetopCluster.info().avg_latency,
            Duration::from_millis(9_999)
        );
        assert_eq!(JobType::JtMissingTxn.limit(), 1_200);
    }

    #[test]
    fn name_matches_reference_string() {
        assert_eq!(JobType::JtAccept.name(), "acceptLedger");
        assert_eq!(JobType::JtClientWebsocket.name(), "clientWebsocket");
    }
}
