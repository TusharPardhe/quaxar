use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum JobType {
    Invalid,
    Pack,
    PubOldLedger,
    Client,
    ClientSubscribe,
    ClientFeeChange,
    ClientConsensus,
    ClientAcctHist,
    ClientRpc,
    ClientWebsocket,
    Rpc,
    Sweep,
    ValidationUt,
    Manifest,
    UpdatePf,
    TransactionL,
    ReplayReq,
    LedgerReq,
    ProposalUt,
    ReplayTask,
    Transaction,
    MissingTxn,
    RequestedTxn,
    Batch,
    LedgerData,
    Advance,
    PubLedger,
    TxnData,
    Wal,
    ValidationT,
    Write,
    Accept,
    ProposalT,
    NetopCluster,
    NetopTimer,
    Admin,
    Peer,
    Disk,
    TxnProc,
    ObSetup,
    PathFind,
    HoRead,
    HoWrite,
    Generic,
    NsSyncRead,
    NsAsyncRead,
    NsWrite,
}

impl JobType {
    pub const fn name(self) -> &'static str {
        match self {
            JobType::Invalid => "invalid",
            JobType::Pack => "makeFetchPack",
            JobType::PubOldLedger => "publishAcqLedger",
            JobType::Client => "clientCommand",
            JobType::ClientSubscribe => "clientSubscribe",
            JobType::ClientFeeChange => "clientFeeChange",
            JobType::ClientConsensus => "clientConsensus",
            JobType::ClientAcctHist => "clientAccountHistory",
            JobType::ClientRpc => "clientRPC",
            JobType::ClientWebsocket => "clientWebsocket",
            JobType::Rpc => "RPC",
            JobType::Sweep => "sweep",
            JobType::ValidationUt => "untrustedValidation",
            JobType::Manifest => "manifest",
            JobType::UpdatePf => "updatePaths",
            JobType::TransactionL => "localTransaction",
            JobType::ReplayReq => "ledgerReplayRequest",
            JobType::LedgerReq => "ledgerRequest",
            JobType::ProposalUt => "untrustedProposal",
            JobType::ReplayTask => "ledgerReplayTask",
            JobType::Transaction => "transaction",
            JobType::MissingTxn => "handleHaveTransactions",
            JobType::RequestedTxn => "doTransactions",
            JobType::Batch => "batch",
            JobType::LedgerData => "ledgerData",
            JobType::Advance => "advanceLedger",
            JobType::PubLedger => "publishNewLedger",
            JobType::TxnData => "fetchTxnData",
            JobType::Wal => "writeAhead",
            JobType::ValidationT => "trustedValidation",
            JobType::Write => "writeObjects",
            JobType::Accept => "acceptLedger",
            JobType::ProposalT => "trustedProposal",
            JobType::NetopCluster => "clusterReport",
            JobType::NetopTimer => "heartbeat",
            JobType::Admin => "administration",
            JobType::Peer => "peerCommand",
            JobType::Disk => "diskAccess",
            JobType::TxnProc => "processTransaction",
            JobType::ObSetup => "orderBookSetup",
            JobType::PathFind => "pathFind",
            JobType::HoRead => "nodeRead",
            JobType::HoWrite => "nodeWrite",
            JobType::Generic => "generic",
            JobType::NsSyncRead => "SyncReadNode",
            JobType::NsAsyncRead => "AsyncReadNode",
            JobType::NsWrite => "WriteNode",
        }
    }
}

impl fmt::Display for JobType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobTypeInfo {
    job_type: JobType,
    name: &'static str,
    limit: i32,
    avg_latency_ms: u64,
    peak_latency_ms: u64,
}

impl JobTypeInfo {
    pub const fn new(
        job_type: JobType,
        name: &'static str,
        limit: i32,
        avg_latency_ms: u64,
        peak_latency_ms: u64,
    ) -> Self {
        Self {
            job_type,
            name,
            limit,
            avg_latency_ms,
            peak_latency_ms,
        }
    }

    pub const fn job_type(&self) -> JobType {
        self.job_type
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn limit(&self) -> i32 {
        self.limit
    }

    pub const fn special(&self) -> bool {
        self.limit == 0
    }

    pub fn average_latency(&self) -> Duration {
        Duration::from_millis(self.avg_latency_ms)
    }

    pub fn peak_latency(&self) -> Duration {
        Duration::from_millis(self.peak_latency_ms)
    }
}

pub struct JobTypes;

impl JobTypes {
    pub fn instance() -> &'static Self {
        static INSTANCE: OnceLock<JobTypes> = OnceLock::new();
        INSTANCE.get_or_init(|| JobTypes)
    }

    pub fn size(&self) -> usize {
        JOB_TYPE_INFOS.len()
    }

    pub fn all(&self) -> &'static [JobTypeInfo] {
        JOB_TYPE_INFOS
    }

    pub fn name(job_type: JobType) -> &'static str {
        job_type.name()
    }

    pub fn get(&self, job_type: JobType) -> JobTypeInfo {
        JOB_TYPE_INFOS
            .iter()
            .copied()
            .find(|info| info.job_type == job_type)
            .unwrap_or(INVALID_JOB_TYPE_INFO)
    }

    pub fn get_invalid(&self) -> JobTypeInfo {
        INVALID_JOB_TYPE_INFO
    }

    pub fn iter(&self) -> impl Iterator<Item = JobTypeInfo> + '_ {
        JOB_TYPE_INFOS.iter().copied()
    }
}

pub const INVALID_JOB_TYPE_INFO: JobTypeInfo =
    JobTypeInfo::new(JobType::Invalid, "invalid", 0, 0, 0);

pub const JOB_TYPE_INFOS: &[JobTypeInfo] = &[
    JobTypeInfo::new(JobType::Pack, "makeFetchPack", 1, 0, 0),
    JobTypeInfo::new(JobType::PubOldLedger, "publishAcqLedger", 2, 10_000, 15_000),
    JobTypeInfo::new(
        JobType::ValidationUt,
        "untrustedValidation",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(JobType::Manifest, "manifest", i32::MAX, 2_000, 5_000),
    JobTypeInfo::new(
        JobType::TransactionL,
        "localTransaction",
        i32::MAX,
        100,
        500,
    ),
    JobTypeInfo::new(JobType::ReplayReq, "ledgerReplayRequest", 10, 250, 1_000),
    JobTypeInfo::new(JobType::LedgerReq, "ledgerRequest", 3, 0, 0),
    JobTypeInfo::new(
        JobType::ProposalUt,
        "untrustedProposal",
        i32::MAX,
        500,
        1_250,
    ),
    JobTypeInfo::new(JobType::ReplayTask, "ledgerReplayTask", i32::MAX, 0, 0),
    JobTypeInfo::new(JobType::LedgerData, "ledgerData", 3, 0, 0),
    JobTypeInfo::new(JobType::Client, "clientCommand", i32::MAX, 2_000, 5_000),
    JobTypeInfo::new(
        JobType::ClientSubscribe,
        "clientSubscribe",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(
        JobType::ClientFeeChange,
        "clientFeeChange",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(
        JobType::ClientConsensus,
        "clientConsensus",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(
        JobType::ClientAcctHist,
        "clientAccountHistory",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(JobType::ClientRpc, "clientRPC", i32::MAX, 2_000, 5_000),
    JobTypeInfo::new(
        JobType::ClientWebsocket,
        "clientWebsocket",
        i32::MAX,
        2_000,
        5_000,
    ),
    JobTypeInfo::new(JobType::Rpc, "RPC", i32::MAX, 0, 0),
    JobTypeInfo::new(JobType::UpdatePf, "updatePaths", 1, 0, 0),
    JobTypeInfo::new(JobType::Transaction, "transaction", i32::MAX, 250, 1_000),
    JobTypeInfo::new(JobType::Batch, "batch", i32::MAX, 250, 1_000),
    JobTypeInfo::new(JobType::Advance, "advanceLedger", i32::MAX, 0, 0),
    JobTypeInfo::new(
        JobType::PubLedger,
        "publishNewLedger",
        i32::MAX,
        3_000,
        4_500,
    ),
    JobTypeInfo::new(JobType::TxnData, "fetchTxnData", 5, 0, 0),
    JobTypeInfo::new(JobType::Wal, "writeAhead", i32::MAX, 1_000, 2_500),
    JobTypeInfo::new(
        JobType::ValidationT,
        "trustedValidation",
        i32::MAX,
        500,
        1_500,
    ),
    JobTypeInfo::new(JobType::Write, "writeObjects", i32::MAX, 1_750, 2_500),
    JobTypeInfo::new(JobType::Accept, "acceptLedger", i32::MAX, 0, 0),
    JobTypeInfo::new(JobType::ProposalT, "trustedProposal", i32::MAX, 100, 500),
    JobTypeInfo::new(JobType::Sweep, "sweep", 1, 0, 0),
    JobTypeInfo::new(JobType::NetopCluster, "clusterReport", 1, 9_999, 9_999),
    JobTypeInfo::new(JobType::NetopTimer, "heartbeat", 1, 999, 999),
    JobTypeInfo::new(JobType::Admin, "administration", i32::MAX, 0, 0),
    JobTypeInfo::new(JobType::MissingTxn, "handleHaveTransactions", 1_200, 0, 0),
    JobTypeInfo::new(JobType::RequestedTxn, "doTransactions", 1_200, 0, 0),
    JobTypeInfo::new(JobType::Peer, "peerCommand", 0, 200, 2_500),
    JobTypeInfo::new(JobType::Disk, "diskAccess", 0, 500, 1_000),
    JobTypeInfo::new(JobType::TxnProc, "processTransaction", 0, 0, 0),
    JobTypeInfo::new(JobType::ObSetup, "orderBookSetup", 0, 0, 0),
    JobTypeInfo::new(JobType::PathFind, "pathFind", 0, 0, 0),
    JobTypeInfo::new(JobType::HoRead, "nodeRead", 0, 0, 0),
    JobTypeInfo::new(JobType::HoWrite, "nodeWrite", 0, 0, 0),
    JobTypeInfo::new(JobType::Generic, "generic", 0, 0, 0),
    JobTypeInfo::new(JobType::NsSyncRead, "SyncReadNode", 0, 0, 0),
    JobTypeInfo::new(JobType::NsAsyncRead, "AsyncReadNode", 0, 0, 0),
    JobTypeInfo::new(JobType::NsWrite, "WriteNode", 0, 0, 0),
];
