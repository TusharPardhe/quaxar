//! NodeStore scheduler shim backed by the app job queue.

use crate::job::job_queue::JobQueue;
use crate::job::job_types::JobType;
use nodestore::{
    BatchWriteReport as NodeStoreBatchWriteReportInner, FetchReport as NodeStoreFetchReportInner,
    FetchType as NodeStoreFetchTypeInner, Scheduler as NodeStoreSchedulerRuntime,
    Task as NodeStoreRuntimeTask,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub trait NodeStoreTask: Send + Sync + 'static {
    fn perform_scheduled_task(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStoreFetchType {
    Synchronous,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeStoreFetchReport {
    pub elapsed: Duration,
    pub fetch_type: NodeStoreFetchType,
    pub was_found: bool,
}

impl NodeStoreFetchReport {
    pub fn new(fetch_type: NodeStoreFetchType) -> Self {
        Self {
            elapsed: Duration::default(),
            fetch_type,
            was_found: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeStoreBatchWriteReport {
    pub elapsed: Duration,
    pub write_count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleTaskResult {
    Queued,
    RanSynchronously,
    DroppedStopped,
}

#[derive(Clone, Debug)]
pub struct NodeStoreScheduler {
    job_queue: JobQueue,
}

impl NodeStoreScheduler {
    pub fn new(job_queue: JobQueue) -> Self {
        Self { job_queue }
    }

    pub fn job_queue(&self) -> &JobQueue {
        &self.job_queue
    }

    pub fn schedule_task(&self, task: Arc<dyn NodeStoreTask>) -> ScheduleTaskResult {
        self.schedule_task_impl(move || task.perform_scheduled_task())
    }

    fn schedule_task_impl(&self, perform: impl FnOnce() + Send + 'static) -> ScheduleTaskResult {
        if self.job_queue.is_stopped() {
            return ScheduleTaskResult::DroppedStopped;
        }

        let perform = Arc::new(Mutex::new(Some(perform)));
        let perform_in_job = Arc::clone(&perform);
        let scheduled = self
            .job_queue
            .add_job(JobType::JtWrite, "NObjStore", move || {
                if let Some(perform) = perform_in_job
                    .lock()
                    .expect("scheduled task mutex must not be poisoned")
                    .take()
                {
                    perform();
                }
            });

        if scheduled {
            ScheduleTaskResult::Queued
        } else {
            if let Some(perform) = perform
                .lock()
                .expect("scheduled task mutex must not be poisoned")
                .take()
            {
                perform();
            }
            ScheduleTaskResult::RanSynchronously
        }
    }

    pub fn on_fetch(&self, report: NodeStoreFetchReport) {
        if self.job_queue.is_stopped() {
            return;
        }

        let job_type = match report.fetch_type {
            NodeStoreFetchType::Async => JobType::JtNsAsyncRead,
            NodeStoreFetchType::Synchronous => JobType::JtNsSyncRead,
        };
        self.job_queue.add_load_events(job_type, 1, report.elapsed);
    }

    pub fn on_batch_write(&self, report: NodeStoreBatchWriteReport) {
        if self.job_queue.is_stopped() {
            return;
        }

        self.job_queue.add_load_events(
            JobType::JtNsWrite,
            report.write_count as u64,
            report.elapsed,
        );
    }
}

impl NodeStoreSchedulerRuntime for NodeStoreScheduler {
    fn schedule_task(&self, task: Arc<dyn NodeStoreRuntimeTask>) {
        let _ = self.schedule_task_impl(move || task.perform_scheduled_task());
    }

    fn on_fetch(&self, report: NodeStoreFetchReportInner) {
        self.on_fetch(NodeStoreFetchReport {
            elapsed: report.elapsed,
            fetch_type: match report.fetch_type {
                NodeStoreFetchTypeInner::Synchronous => NodeStoreFetchType::Synchronous,
                NodeStoreFetchTypeInner::Async => NodeStoreFetchType::Async,
            },
            was_found: report.was_found,
        });
    }

    fn on_batch_write(&self, report: NodeStoreBatchWriteReportInner) {
        self.on_batch_write(NodeStoreBatchWriteReport {
            elapsed: report.elapsed,
            write_count: report.write_count,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NodeStoreBatchWriteReport, NodeStoreFetchReport, NodeStoreFetchType, NodeStoreScheduler,
        NodeStoreTask, ScheduleTaskResult,
    };
    use crate::job::job_queue::JobQueue;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingTask(AtomicUsize);

    impl NodeStoreTask for RecordingTask {
        fn perform_scheduled_task(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn node_store_scheduler_queues_tasks_and_records_io_load() {
        let queue = JobQueue::default();
        let scheduler = NodeStoreScheduler::new(queue.clone());
        let task = Arc::new(RecordingTask::default());

        assert_eq!(
            scheduler.schedule_task(task.clone()),
            ScheduleTaskResult::Queued
        );
        queue.run_until_idle();
        assert_eq!(task.0.load(Ordering::Relaxed), 1);

        scheduler.on_fetch(NodeStoreFetchReport {
            elapsed: Duration::from_millis(3),
            fetch_type: NodeStoreFetchType::Async,
            was_found: true,
        });
        scheduler.on_batch_write(NodeStoreBatchWriteReport {
            elapsed: Duration::from_millis(7),
            write_count: 2,
        });

        let json = queue.get_json(0);
        assert!(
            json["jobs"]
                .as_array()
                .expect("jobs should be an array")
                .iter()
                .any(|entry| entry["job_type"] == "AsyncReadNode")
        );
    }
}
