//! App-local job type mirror.
//!
//! This crate re-exports the synced workspace `perflog` job table so the
//! runtime queue slice uses the the reference implementation ordering and per-type limits
//! instead of duplicating a stale enum list.

pub use perflog::{INVALID_JOB_TYPE_INFO, JOB_TYPE_INFOS, JobType, JobTypeInfo, JobTypes};
