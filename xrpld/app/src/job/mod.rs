//! Job scheduling: a persistent worker-thread pool that executes
//! prioritized, concurrency-limited jobs. See [`job_queue`] and
//! [`job_types`].

pub mod job_queue;
pub mod job_types;

pub use job_queue::{JobQueue, RunningJob};
pub use job_types::{JobType, JobTypeInfo};
