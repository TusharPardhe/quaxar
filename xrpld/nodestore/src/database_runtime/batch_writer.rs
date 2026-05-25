use crate::{
    Batch, BatchWriteReport, NodeObject, Scheduler, Task, batch_write_limit_size,
    batch_write_preallocation_size,
};
use std::cmp::max;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

struct BatchWriterState {
    write_set: Batch,
    write_load: usize,
    write_pending: bool,
}

impl BatchWriterState {
    fn new() -> Self {
        Self {
            write_set: Vec::with_capacity(batch_write_preallocation_size),
            write_load: 0,
            write_pending: false,
        }
    }
}

pub struct BatchWriter {
    callback: Box<dyn Fn(&Batch) + Send + Sync>,
    scheduler: Arc<dyn Scheduler>,
    state: Mutex<BatchWriterState>,
    condition: Condvar,
}

impl BatchWriter {
    fn lock_state(&self) -> MutexGuard<'_, BatchWriterState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn wait_state<'a>(
        &self,
        state: MutexGuard<'a, BatchWriterState>,
    ) -> MutexGuard<'a, BatchWriterState> {
        match self.condition.wait(state) {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn new<F>(callback: F, scheduler: Arc<dyn Scheduler>) -> Arc<Self>
    where
        F: Fn(&Batch) + Send + Sync + 'static,
    {
        Arc::new(Self {
            callback: Box::new(callback),
            scheduler,
            state: Mutex::new(BatchWriterState::new()),
            condition: Condvar::new(),
        })
    }

    pub fn store(self: &Arc<Self>, object: Arc<NodeObject>) {
        let should_schedule = {
            let mut state = self.lock_state();
            // Match reference the reference source: writers only block while the queued
            // batch itself is full. Once the scheduled task swaps that batch
            // out, a new batch may grow while the prior one is still flushing.
            while state.write_set.len() >= batch_write_limit_size {
                state = self.wait_state(state);
            }

            state.write_set.push(object);
            if state.write_pending {
                false
            } else {
                state.write_pending = true;
                true
            }
        };

        if should_schedule {
            let task: Arc<dyn Task> = Arc::clone(self) as Arc<dyn Task>;
            self.scheduler.schedule_task(task);
        }
    }

    pub fn get_write_load(&self) -> i32 {
        let state = self.lock_state();
        max(state.write_load, state.write_set.len()) as i32
    }

    pub fn wait_for_writing(&self) {
        let mut state = self.lock_state();
        while state.write_pending {
            state = self.wait_state(state);
        }
    }

    fn write_batch(&self) {
        let panic_payload = loop {
            let batch = {
                let mut state = self.lock_state();

                if state.write_set.is_empty() {
                    state.write_pending = false;
                    self.condition.notify_all();
                    return;
                }

                let mut batch = Vec::with_capacity(batch_write_preallocation_size);
                std::mem::swap(&mut state.write_set, &mut batch);
                state.write_load = batch.len();
                batch
            };

            let before = Instant::now();
            match catch_unwind(AssertUnwindSafe(|| {
                (self.callback)(&batch);
                self.scheduler.on_batch_write(BatchWriteReport {
                    elapsed: std::time::Duration::from_millis(before.elapsed().as_millis() as u64),
                    write_count: batch.len() as i32,
                });
            })) {
                Ok(()) => {}
                Err(payload) => {
                    break Some(payload);
                }
            }
        };

        let mut state = self.lock_state();
        state.write_pending = false;
        self.condition.notify_all();

        if let Some(payload) = panic_payload {
            resume_unwind(payload);
        }
    }
}

impl Task for BatchWriter {
    fn perform_scheduled_task(&self) {
        self.write_batch();
    }
}

impl Drop for BatchWriter {
    fn drop(&mut self) {
        self.wait_for_writing();
    }
}

#[cfg(test)]
mod tests {
    use super::BatchWriter;
    use crate::{DummyScheduler, NodeObject, NodeObjectType, Scheduler};
    use basics::base_uint::Uint256;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CountingScheduler {
        scheduled: AtomicUsize,
        task: Mutex<Option<Arc<dyn crate::Task>>>,
    }

    impl Scheduler for CountingScheduler {
        fn schedule_task(&self, task: Arc<dyn crate::Task>) {
            self.scheduled.fetch_add(1, Ordering::Relaxed);
            *self.task.lock().expect("task mutex") = Some(task);
        }

        fn on_fetch(&self, _report: crate::FetchReport) {}

        fn on_batch_write(&self, _report: crate::BatchWriteReport) {}
    }

    fn sample_object(fill: u8) -> Arc<NodeObject> {
        NodeObject::create_object(
            NodeObjectType::Ledger,
            vec![fill],
            Uint256::from_array([fill; 32]),
        )
    }

    #[test]
    fn batch_writer_flushes_pending_items_and_reports_zero_load_after_write() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&written);
        let scheduler: Arc<dyn Scheduler> = Arc::new(DummyScheduler);
        let writer = BatchWriter::new(
            move |batch| {
                seen.lock()
                    .expect("written mutex")
                    .extend(batch.iter().map(|object| *object.hash()));
            },
            scheduler,
        );

        writer.store(sample_object(0x11));
        writer.store(sample_object(0x22));
        writer.wait_for_writing();

        let hashes = written.lock().expect("written mutex");
        assert_eq!(hashes.len(), 2);
        assert_eq!(writer.get_write_load(), 1);
    }

    #[test]
    fn batch_writer_only_schedules_once_while_a_write_is_pending() {
        let scheduler = Arc::new(CountingScheduler::default());
        let writer = BatchWriter::new(|_batch| {}, Arc::clone(&scheduler) as Arc<dyn Scheduler>);

        writer.store(sample_object(0x33));
        writer.store(sample_object(0x44));

        assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 1);
        scheduler
            .task
            .lock()
            .expect("task mutex")
            .take()
            .expect("scheduled task")
            .perform_scheduled_task();
        writer.wait_for_writing();
    }

    #[test]
    fn batch_writer_clears_pending_state_after_callback_panics() {
        let scheduler = Arc::new(CountingScheduler::default());
        let writer = BatchWriter::new(
            |_batch| panic!("batch callback panic"),
            Arc::clone(&scheduler) as Arc<dyn Scheduler>,
        );

        writer.store(sample_object(0x55));
        assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 1);

        let task = scheduler
            .task
            .lock()
            .expect("task mutex")
            .take()
            .expect("scheduled task");
        let result = catch_unwind(AssertUnwindSafe(|| task.perform_scheduled_task()));
        assert!(result.is_err());

        writer.wait_for_writing();
        writer.store(sample_object(0x66));
        assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn batch_writer_drop_survives_panicking_callback_path() {
        let scheduler = Arc::new(CountingScheduler::default());
        let writer = BatchWriter::new(
            |_batch| panic!("batch callback panic"),
            Arc::clone(&scheduler) as Arc<dyn Scheduler>,
        );

        writer.store(sample_object(0x77));
        let task = scheduler
            .task
            .lock()
            .expect("task mutex")
            .take()
            .expect("scheduled task");
        let result = catch_unwind(AssertUnwindSafe(|| task.perform_scheduled_task()));
        assert!(result.is_err());

        let result = catch_unwind(AssertUnwindSafe(|| drop(writer)));
        assert!(result.is_ok());
    }
}
