use basics::base_uint::Uint256;
use nodestore::{
    BATCH_WRITE_LIMIT_SIZE, Batch, BatchWriteReport, BatchWriter, NodeObject, NodeObjectType,
    Scheduler, Task,
};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

fn sample_object(fill: u8) -> Arc<NodeObject> {
    NodeObject::create_object(
        NodeObjectType::Ledger,
        vec![fill],
        Uint256::from_array([fill; 32]),
    )
}

#[derive(Default)]
struct ManualScheduler {
    scheduled: AtomicUsize,
    tasks: Mutex<VecDeque<Arc<dyn Task>>>,
    task_ready: Condvar,
    batch_reports: Mutex<Vec<BatchWriteReport>>,
}

impl ManualScheduler {
    fn take_task(&self) -> Arc<dyn Task> {
        let mut tasks = self.tasks.lock().expect("task queue mutex");
        while tasks.is_empty() {
            tasks = self.task_ready.wait(tasks).expect("task queue mutex");
        }
        tasks.pop_front().expect("scheduled task")
    }
}

impl Scheduler for ManualScheduler {
    fn schedule_task(&self, task: Arc<dyn Task>) {
        self.scheduled.fetch_add(1, Ordering::Relaxed);
        self.tasks.lock().expect("task queue mutex").push_back(task);
        self.task_ready.notify_one();
    }

    fn on_fetch(&self, _report: nodestore::FetchReport) {}

    fn on_batch_write(&self, report: BatchWriteReport) {
        self.batch_reports
            .lock()
            .expect("batch report mutex")
            .push(report);
    }
}

#[test]
fn batch_writer_allows_the_next_batch_to_grow_while_the_prior_flush_is_in_flight() {
    let scheduler = Arc::new(ManualScheduler::default());
    let written = Arc::new(Mutex::new(Vec::<usize>::new()));
    let seen = Arc::clone(&written);
    let release_flush = Arc::new((Mutex::new(false), Condvar::new()));
    let release_flush_in_callback = Arc::clone(&release_flush);
    let writer = BatchWriter::new(
        move |batch: &Batch| {
            let (released, condvar) = &*release_flush_in_callback;
            let mut released = released.lock().expect("release mutex");
            seen.lock().expect("written mutex").push(batch.len());
            while !*released {
                released = condvar.wait(released).expect("release mutex");
            }
        },
        Arc::clone(&scheduler) as Arc<dyn Scheduler>,
    );

    for index in 0..BATCH_WRITE_LIMIT_SIZE {
        writer.store(sample_object((index & 0xFF) as u8));
    }
    assert_eq!(writer.get_write_load(), BATCH_WRITE_LIMIT_SIZE as i32);
    assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 1);

    let scheduled_task = scheduler.take_task();
    let (started_tx, started_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).expect("flush-start send");
        scheduled_task.perform_scheduled_task();
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("flush should start");

    let next_batch_writer = Arc::clone(&writer);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        next_batch_writer.store(sample_object(0xAA));
        tx.send(()).expect("signal send");
    });

    rx.recv_timeout(Duration::from_secs(2))
        .expect("next batch should keep growing while the first flush is running");
    assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 1);

    let (released, condvar) = &*release_flush;
    *released.lock().expect("release mutex") = true;
    condvar.notify_all();
    worker.join().expect("flush worker");
    writer.wait_for_writing();

    assert_eq!(
        written.lock().expect("written mutex").as_slice(),
        &[BATCH_WRITE_LIMIT_SIZE, 1]
    );
    let reports = scheduler.batch_reports.lock().expect("batch report mutex");
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].write_count, BATCH_WRITE_LIMIT_SIZE as i32);
    assert_eq!(reports[1].write_count, 1);
}

#[test]
fn batch_writer_recovers_from_panicking_callbacks_without_stalling_followup_writes() {
    let scheduler = Arc::new(ManualScheduler::default());
    let writer = BatchWriter::new(
        |_batch| panic!("batch callback panic"),
        Arc::clone(&scheduler) as Arc<dyn Scheduler>,
    );

    writer.store(sample_object(0xAB));
    let task = scheduler.take_task();
    let result = catch_unwind(AssertUnwindSafe(|| task.perform_scheduled_task()));
    assert!(result.is_err());

    writer.wait_for_writing();
    writer.store(sample_object(0xBC));
    assert_eq!(scheduler.scheduled.load(Ordering::Relaxed), 2);
}
