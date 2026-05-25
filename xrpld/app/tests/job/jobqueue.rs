use app::{JobQueue, JobType};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

#[test]
fn job_queue_prefers_higher_priority_waiting_jobs_in_the_same_window() {
    let queue = JobQueue::new();
    let order = Arc::new(Mutex::new(Vec::new()));

    let low_order = Arc::clone(&order);
    assert!(queue.add_job(JobType::Pack, "low", move || {
        low_order.lock().expect("order mutex poisoned").push("pack");
    }));

    let high_order = Arc::clone(&order);
    assert!(queue.add_job(JobType::Accept, "high", move || {
        high_order
            .lock()
            .expect("order mutex poisoned")
            .push("accept");
    }));

    queue.run_until_idle();
    assert_eq!(
        order.lock().expect("order mutex poisoned").as_slice(),
        &["accept", "pack"]
    );
}

#[test]
fn job_queue_counts_waiting_and_running_jobs_per_type() {
    let queue = JobQueue::new();

    assert!(queue.add_job(JobType::Pack, "pack-1", || {}));
    assert!(queue.add_job(JobType::Pack, "pack-2", || {}));
    assert!(queue.add_job(JobType::Accept, "accept-1", || {}));

    assert_eq!(queue.get_job_count(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_ge(JobType::Pack), 3);
    assert_eq!(queue.get_job_count_ge(JobType::Accept), 1);

    let first = queue
        .reserve_next_job()
        .expect("one job should be runnable");
    assert_eq!(first.job_type(), JobType::Accept);
    assert_eq!(queue.get_job_count(JobType::Accept), 0);
    assert_eq!(queue.get_job_count_total(JobType::Accept), 1);
    assert_eq!(queue.get_job_count(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_ge(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_ge(JobType::Accept), 0);

    first.finish();

    let second = queue.reserve_next_job().expect("pack job should now run");
    assert_eq!(second.job_type(), JobType::Pack);
    assert_eq!(queue.get_job_count(JobType::Pack), 1);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 2);
    second.finish();
}

#[test]
fn job_queue_stop_waits_for_running_and_queued_work_to_drain() {
    let queue = JobQueue::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let low_order = Arc::clone(&order);
    assert!(queue.add_job(JobType::Pack, "low", move || {
        low_order.lock().expect("order mutex poisoned").push("pack");
    }));

    let high_order = Arc::clone(&order);
    assert!(queue.add_job(JobType::Accept, "high", move || {
        started_tx.send(()).expect("start signal should send");
        release_rx.recv().expect("release signal should arrive");
        high_order
            .lock()
            .expect("order mutex poisoned")
            .push("accept");
    }));

    let worker_queue = queue.clone();
    let worker = thread::spawn(move || while worker_queue.dispatch_next_job().is_some() {});

    started_rx.recv().expect("high-priority job should start");

    let stop_queue = queue.clone();
    let stopper = thread::spawn(move || {
        stop_queue.stop();
    });

    while !queue.is_stopping() {
        thread::yield_now();
    }

    release_tx.send(()).expect("release signal should send");

    stopper.join().expect("stopper should finish");
    worker.join().expect("worker should finish");

    assert!(queue.is_stopped());
    assert_eq!(queue.get_job_count(JobType::Pack), 0);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 0);
    assert_eq!(queue.get_job_count(JobType::Accept), 0);
    assert_eq!(queue.get_job_count_total(JobType::Accept), 0);
    assert_eq!(
        order.lock().expect("order mutex poisoned").as_slice(),
        &["accept", "pack"]
    );
}

#[test]
fn job_queue_limit_zero_types_are_not_runnable() {
    let queue = JobQueue::new();
    assert!(queue.add_job(JobType::Peer, "peer", || {}));
    assert_eq!(queue.get_job_count(JobType::Peer), 1);
    assert_eq!(queue.get_job_count_total(JobType::Peer), 1);
    assert!(queue.reserve_next_job().is_none());
}

#[test]
fn job_queue_running_job_is_fully_owned_by_the_reservation() {
    let queue = JobQueue::new();
    assert!(queue.add_job(JobType::Pack, "pack", || {}));

    let running = queue.reserve_next_job().expect("job should be reserved");
    assert_eq!(running.job_type(), JobType::Pack);
    assert_eq!(running.name(), "pack");
    assert_eq!(running.index(), 1);
    assert!(running.queue_time() <= Instant::now());
    assert_eq!(queue.get_job_count(JobType::Pack), 0);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 1);

    drop(running);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 0);
}
