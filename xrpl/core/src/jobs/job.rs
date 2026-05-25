use crate::closure_counter::ClosureCounter;
use std::cmp::Ordering;
use std::fmt;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum JobType {
    Invalid = -1,
    Pack = 0,
    PubOldLedger = 1,
    Client = 2,
    ClientSubscribe = 3,
    ClientFeeChange = 4,
    ClientConsensus = 5,
    ClientAcctHist = 6,
    ClientRpc = 7,
    ClientWebsocket = 8,
    Rpc = 9,
    Sweep = 10,
    ValidationUt = 11,
    Manifest = 12,
    UpdatePf = 13,
    TransactionL = 14,
    ReplayReq = 15,
    LedgerReq = 16,
    ProposalUt = 17,
    ReplayTask = 18,
    Transaction = 19,
    MissingTxn = 20,
    RequestedTxn = 21,
    Batch = 22,
    LedgerData = 23,
    Advance = 24,
    PubLedger = 25,
    TxnData = 26,
    Wal = 27,
    ValidationT = 28,
    Write = 29,
    Accept = 30,
    ProposalT = 31,
    NetopCluster = 32,
    NetopTimer = 33,
    Admin = 34,
    Peer = 35,
    Disk = 36,
    TxnProc = 37,
    ObSetup = 38,
    PathFind = 39,
    HoRead = 40,
    HoWrite = 41,
    Generic = 42,
    NsSyncRead = 43,
    NsAsyncRead = 44,
    NsWrite = 45,
}

pub type JobCounter = ClosureCounter<fn()>;

pub struct Job {
    job_type: JobType,
    job_index: u64,
    job: Option<Box<dyn FnMut() + Send + 'static>>,
    name: String,
    queue_time: Option<Instant>,
}

impl fmt::Debug for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job")
            .field("job_type", &self.job_type)
            .field("job_index", &self.job_index)
            .field("name", &self.name)
            .field("has_job", &self.job.is_some())
            .field("queue_time", &self.queue_time)
            .finish()
    }
}

impl Default for Job {
    fn default() -> Self {
        Self::new(JobType::Invalid, 0)
    }
}

impl Job {
    pub fn new(job_type: JobType, job_index: u64) -> Self {
        Self {
            job_type,
            job_index,
            job: None,
            name: String::new(),
            queue_time: None,
        }
    }

    pub fn new_with_closure<F>(
        job_type: JobType,
        name: impl Into<String>,
        job_index: u64,
        job: F,
    ) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        Self {
            job_type,
            job_index,
            job: Some(Box::new(job)),
            name: name.into(),
            queue_time: Some(Instant::now()),
        }
    }

    pub const fn get_type(&self) -> JobType {
        self.job_type
    }

    pub const fn job_index(&self) -> u64 {
        self.job_index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn queue_time(&self) -> Option<Instant> {
        self.queue_time
    }

    pub fn do_job(&mut self) {
        if let Some(mut job) = self.job.take() {
            job();
        }
    }
}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.job_type < other.job_type {
            Ordering::Greater
        } else if self.job_type > other.job_type {
            Ordering::Less
        } else {
            self.job_index.cmp(&other.job_index)
        }
    }
}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.job_type == other.job_type && self.job_index == other.job_index
    }
}

impl Eq for Job {}
