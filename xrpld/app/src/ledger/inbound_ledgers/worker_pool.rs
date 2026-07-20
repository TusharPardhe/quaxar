//! Fixed-size thread pool for acquisition job ticks.
//!
//! Worker threads pull short-lived jobs from a shared queue. Each job processes
//! one tick of one acquisition and returns — the thread immediately picks up
//! the next job for ANY acquisition.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// Shared job queue type used by both the pool and AcquisitionState for self-resubmission.
pub type JobQueue = Arc<(Mutex<VecDeque<Box<dyn FnOnce() + Send>>>, Condvar)>;

/// A fixed-size thread pool for acquisition job ticks.
///
/// Worker threads pull short-lived jobs from a shared queue. Each job processes
/// one tick of one acquisition and returns — the thread immediately picks up
/// the next job for ANY acquisition.
pub struct WorkerPool {
    queue: JobQueue,
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl WorkerPool {
    /// Create a new worker pool with the given number of threads.
    pub fn new(size: usize) -> Self {
        let queue: JobQueue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let mut workers = Vec::with_capacity(size);
        for i in 0..size {
            let q = Arc::clone(&queue);
            let s = Arc::clone(&stop);
            let handle = thread::Builder::new()
                .name(format!("xrpld-acq-pool-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let (lock, cvar) = &*q;
                            let mut jobs = lock.lock().expect("work pool lock");
                            while jobs.is_empty() {
                                if s.load(Ordering::Acquire) {
                                    return;
                                }
                                jobs = cvar.wait(jobs).expect("work pool cvar wait");
                            }
                            if s.load(Ordering::Acquire) {
                                return;
                            }
                            jobs.pop_front()
                        };
                        if let Some(job) = job {
                            job();
                        }
                    }
                })
                .expect("acquisition pool thread should spawn");
            workers.push(handle);
        }

        Self {
            queue,
            stop,
            workers: Mutex::new(workers),
        }
    }

    /// Submit a job to the pool for execution.
    pub fn submit(&self, job: Box<dyn FnOnce() + Send>) {
        let (lock, cvar) = &*self.queue;
        let mut jobs = lock.lock().expect("work pool submit lock");
        jobs.push_back(job);
        cvar.notify_one();
    }

    /// Get a clone of the job queue for direct submission (used by AcquisitionState).
    pub fn queue(&self) -> JobQueue {
        Arc::clone(&self.queue)
    }

    /// Signal all threads to stop and wait for them to exit.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let (_, cvar) = &*self.queue;
        cvar.notify_all();
        // Join workers
        let mut workers = self.workers.lock().expect("workers lock");
        for handle in workers.drain(..) {
            let _ = handle.join();
        }
    }

    /// Check if the pool has been signaled to stop.
    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        if !self.stop.load(Ordering::Acquire) {
            self.stop.store(true, Ordering::Release);
            let (_, cvar) = &*self.queue;
            cvar.notify_all();
        }
        let mut workers = self.workers.lock().expect("workers lock in drop");
        for handle in workers.drain(..) {
            let _ = handle.join();
        }
    }
}
