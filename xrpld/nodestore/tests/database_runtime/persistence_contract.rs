use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    ASYNC_READ_WORK_QUEUE_OVERHEAD_BYTES, AsyncReadWork, DatabaseDelegate, DatabaseRuntime,
    FetchReport, NodeObject, NodeStoreJournal, NullJournal, PersistenceWork, RealScheduler,
    Scheduler, Task,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

#[derive(Default)]
struct RejectingScheduler;

impl Scheduler for RejectingScheduler {
    fn schedule_task(&self, _task: Arc<dyn nodestore::Task>) {}

    fn try_schedule_task(&self, _task: Arc<dyn nodestore::Task>) -> bool {
        false
    }

    fn on_fetch(&self, _report: nodestore::FetchReport) {}

    fn on_batch_write(&self, _report: nodestore::BatchWriteReport) {}
}

struct EmptyDelegate;

impl DatabaseDelegate for EmptyDelegate {
    fn is_same_db(&self, _first: u32, _second: u32) -> bool {
        true
    }

    fn fetch_node_object(
        &self,
        _hash: &Uint256,
        _ledger_seq: u32,
        _fetch_report: &mut FetchReport,
        _duplicate: bool,
        _journal: &dyn NodeStoreJournal,
    ) -> Option<Arc<NodeObject>> {
        None
    }
}

struct DropTicket(Arc<AtomicUsize>);

impl Drop for DropTicket {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// Test-only concrete persistence record. Its payload declaration is the
/// retained vector allocation itself, never an erased callback shell.
struct CountingWriteWork {
    counter: Arc<AtomicUsize>,
    payload: Vec<u8>,
}

impl PersistenceWork for CountingWriteWork {
    fn retained_payload_bytes(&self) -> usize {
        self.payload.capacity()
    }

    fn run(self: Box<Self>) {
        self.counter.fetch_add(1, Ordering::AcqRel);
    }
}

struct TicketWriteWork {
    ticket: DropTicket,
    payload: Vec<u8>,
}

impl PersistenceWork for TicketWriteWork {
    fn retained_payload_bytes(&self) -> usize {
        self.payload.capacity()
    }

    fn run(self: Box<Self>) {
        drop(self);
    }
}

#[test]
fn rejected_write_task_drops_its_owner_ticket_exactly_once() {
    let database = DatabaseRuntime::new(
        Arc::new(EmptyDelegate),
        Arc::new(RejectingScheduler),
        1,
        &Section::new("node_db"),
        Arc::new(NullJournal),
    )
    .expect("database");
    let settled = Arc::new(AtomicUsize::new(0));
    let ticket = DropTicket(Arc::clone(&settled));

    database.schedule_write(Box::new(TicketWriteWork {
        ticket,
        payload: Vec::with_capacity(1),
    }));

    assert_eq!(
        settled.load(Ordering::Acquire),
        1,
        "rejected scheduling must release the persistence owner once"
    );
    database.stop();
}

struct BlockingDelegate {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl DatabaseDelegate for BlockingDelegate {
    fn is_same_db(&self, _first: u32, _second: u32) -> bool {
        true
    }

    fn fetch_node_object(
        &self,
        _hash: &Uint256,
        _ledger_seq: u32,
        _fetch_report: &mut FetchReport,
        _duplicate: bool,
        _journal: &dyn NodeStoreJournal,
    ) -> Option<Arc<NodeObject>> {
        self.entered.send(()).expect("read started");
        self.release
            .lock()
            .expect("release mutex")
            .recv()
            .expect("release read");
        None
    }
}

#[test]
fn default_read_budget_is_derived_from_read_threads_and_request_bundle() {
    let database = DatabaseRuntime::new(
        Arc::new(EmptyDelegate),
        Arc::new(nodestore::DummyScheduler),
        2,
        &Section::new("node_db"),
        Arc::new(NullJournal),
    )
    .expect("database");

    let snapshot = database.read_queue_snapshot();
    assert_eq!(snapshot.key_limit, 8, "2 workers times default rq_bundle 4");
    assert_eq!(snapshot.callback_limit, 8);
    database.stop();
}

struct CountingReadWork(Arc<AtomicUsize>);

impl AsyncReadWork for CountingReadWork {
    fn complete(&mut self, _result: Option<Arc<NodeObject>>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

struct ChannelReadWork(mpsc::Sender<bool>);

impl AsyncReadWork for ChannelReadWork {
    fn complete(&mut self, result: Option<Arc<NodeObject>>) {
        self.0
            .send(result.is_some())
            .expect("typed read work terminal result");
    }
}

struct PaddedReadWork {
    settled: Arc<AtomicUsize>,
    padding: [u8; 64],
}

impl AsyncReadWork for PaddedReadWork {
    fn complete(&mut self, _result: Option<Arc<NodeObject>>) {
        let _ = self.padding;
        self.settled.fetch_add(1, Ordering::AcqRel);
    }
}

struct GateTask {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Task for GateTask {
    fn perform_scheduled_task(&self) {
        self.entered.send(()).expect("gate task should start");
        self.release
            .lock()
            .expect("gate release mutex")
            .recv()
            .expect("gate task should be released");
    }
}

#[test]
fn real_scheduler_rejection_settles_the_rejected_persistence_owner_exactly_once() {
    let scheduler = Arc::new(RealScheduler::new(1));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    assert!(scheduler.try_schedule_task(Arc::new(GateTask {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    })));
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("scheduler worker must be occupied");

    let database = DatabaseRuntime::new(
        Arc::new(EmptyDelegate),
        Arc::clone(&scheduler) as Arc<dyn Scheduler>,
        1,
        &Section::new("node_db"),
        Arc::new(NullJournal),
    )
    .expect("database");
    let accepted = Arc::new(AtomicUsize::new(0));
    let rejected = Arc::new(AtomicUsize::new(0));

    database.schedule_write(Box::new(CountingWriteWork {
        counter: Arc::clone(&accepted),
        payload: Vec::with_capacity(1),
    }));
    database.schedule_write(Box::new(TicketWriteWork {
        ticket: DropTicket(Arc::clone(&rejected)),
        payload: Vec::with_capacity(1),
    }));

    assert_eq!(
        scheduler.pending_task_count(),
        1,
        "worker-derived queue is full"
    );
    assert_eq!(scheduler.task_metrics(), (1, 1));
    assert_eq!(rejected.load(Ordering::Acquire), 1);
    assert_eq!(accepted.load(Ordering::Acquire), 0);

    release_tx.send(()).expect("release scheduler worker");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while accepted.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(accepted.load(Ordering::Acquire), 1);
    assert_eq!(rejected.load(Ordering::Acquire), 1);
    database.stop();
}

#[test]
fn database_stop_settles_each_queued_callback_once_and_late_worker_completion_does_not_repeat_it() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let database = Arc::new(
        DatabaseRuntime::new(
            Arc::new(BlockingDelegate {
                entered: entered_tx,
                release: Mutex::new(release_rx),
            }),
            Arc::new(nodestore::DummyScheduler),
            1,
            &Section::new("node_db"),
            Arc::new(NullJournal),
        )
        .expect("database"),
    );
    let first_settled = Arc::new(AtomicUsize::new(0));
    let queued_settled = Arc::new(AtomicUsize::new(0));

    database.async_fetch(
        Uint256::from_array([0xC1; 32]),
        1,
        Box::new(CountingReadWork(Arc::clone(&first_settled))),
    );
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first read must be active");
    database.async_fetch(
        Uint256::from_array([0xC2; 32]),
        1,
        Box::new(CountingReadWork(Arc::clone(&queued_settled))),
    );

    let stopping_database = Arc::clone(&database);
    let stopping = std::thread::spawn(move || stopping_database.stop());
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while queued_settled.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(queued_settled.load(Ordering::Acquire), 1);

    release_tx.send(()).expect("release active read");
    stopping.join().expect("database stop must complete");
    assert_eq!(first_settled.load(Ordering::Acquire), 1);
    assert_eq!(queued_settled.load(Ordering::Acquire), 1);
}

#[test]
fn callback_budget_rejects_new_work_with_one_cancelled_terminal_result() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut config = Section::new("node_db");
    config.set("read_queue_max_keys", "1");
    config.set("read_queue_max_callbacks", "1");
    let database = DatabaseRuntime::new(
        Arc::new(BlockingDelegate {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }),
        Arc::new(nodestore::DummyScheduler),
        1,
        &config,
        Arc::new(NullJournal),
    )
    .expect("database");
    let first_hash = Uint256::from_array([0xA1; 32]);
    let second_hash = Uint256::from_array([0xB2; 32]);
    let (first_tx, first_rx) = mpsc::channel();
    let (second_tx, second_rx) = mpsc::channel();

    database.async_fetch(first_hash, 1, Box::new(ChannelReadWork(first_tx)));
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first request must own the single callback slot");

    database.async_fetch(second_hash, 1, Box::new(ChannelReadWork(second_tx)));
    assert_eq!(
        second_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("rejected request must settle"),
        false
    );
    let snapshot = database.read_queue_snapshot();
    assert_eq!(snapshot.callback_limit, 1);
    assert_eq!(snapshot.outstanding_callbacks, 1);
    assert_eq!(snapshot.rejected, 1);
    assert_eq!(snapshot.cancelled, 1);

    release_tx.send(()).expect("release first read");
    assert_eq!(
        first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first request terminal result"),
        false
    );
    assert_eq!(database.read_queue_snapshot().outstanding_callbacks, 0);
    database.stop();
}

#[test]
fn byte_permit_rejects_exactly_at_capacity_and_releases_after_terminal_settlement() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut config = Section::new("node_db");
    config.set("read_queue_max_keys", "2");
    config.set("read_queue_max_callbacks", "2");
    let one_work_bytes =
        std::mem::size_of::<PaddedReadWork>() + ASYNC_READ_WORK_QUEUE_OVERHEAD_BYTES;
    config.set("read_queue_max_bytes", one_work_bytes.to_string());
    let database = DatabaseRuntime::new(
        Arc::new(BlockingDelegate {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }),
        Arc::new(nodestore::DummyScheduler),
        1,
        &config,
        Arc::new(NullJournal),
    )
    .expect("database");
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));

    database.async_fetch(
        Uint256::from_array([0xD1; 32]),
        1,
        Box::new(PaddedReadWork {
            settled: Arc::clone(&first),
            padding: [0; 64],
        }),
    );
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first read");
    let admitted = database.read_queue_snapshot();
    assert_eq!(admitted.outstanding_callbacks, 1);
    assert_eq!(admitted.outstanding_bytes, admitted.byte_limit);

    database.async_fetch(
        Uint256::from_array([0xD2; 32]),
        1,
        Box::new(PaddedReadWork {
            settled: Arc::clone(&second),
            padding: [0; 64],
        }),
    );
    assert_eq!(
        second.load(Ordering::Acquire),
        1,
        "over-budget work is terminally rejected"
    );
    assert_eq!(
        database.read_queue_snapshot().outstanding_bytes,
        admitted.byte_limit
    );

    release_tx.send(()).expect("release read");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while first.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(first.load(Ordering::Acquire), 1);
    let released = database.read_queue_snapshot();
    assert_eq!(released.outstanding_callbacks, 0);
    assert_eq!(
        released.outstanding_bytes, 0,
        "completion releases the runtime permit once"
    );
    database.stop();
}
