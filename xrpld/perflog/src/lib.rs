//! Rust parity crate for the reference `xrpl::perf::PerfLog` surface.
//!
//! The reference implementation keeps performance counters, current activity
//! tracking, config-driven log file setup, and a periodic JSON report writer.
//! This crate ports that behavior behind explicit test seams so the core logic
//! stays aligned with the reference reference while the missing `Application`
//! integration remains caller-owned.

mod runtime;
mod support;

pub use runtime::perf_log;
pub use runtime::setup;
pub use support::job_types;
pub use support::journal;

pub use job_types::{INVALID_JOB_TYPE_INFO, JOB_TYPE_INFOS, JobType, JobTypeInfo, JobTypes};
pub use journal::{JournalLevel, NullJournal, PerfLogJournal};
pub use perf_log::{
    NullReportSource, PerfLog, PerfLogImp, PerfLogReportSource, make_perf_log,
    measure_duration_and_log,
};
pub use setup::{PerfLogSetup, setup_perf_log};
