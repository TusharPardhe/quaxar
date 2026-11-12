//! Drop large values off the consensus thread.
//!
//! The resident-tree trim detaches deep SHAMap subtrees every 64 ledgers.
//! Dropping them frees millions of nodes, which took up to several seconds on
//! the NetworkOps strand (the thread that also runs consensus), so a round
//! could stall (mainnet, Nov 12 02:46 UTC: 6.85 s, three ledgers missed).
//!
//! IMPORTANT: the strand only detaches (moves the child links out); this
//! thread does the freeing at a low CPU priority and then asks jemalloc to
//! return the freed pages to the OS.

use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Instant;

/// Thread name, also used by tests.
pub const DROPPER_THREAD_NAME: &str = "quaxar-graph-drop";
/// Pending jobs before `submit` falls back to dropping on the caller. Trims
/// come every 64 ledgers; acquired-graph releases can burst during catch-up.
/// Queued subtrees stay in memory until dropped, so the depth stays bounded.
const QUEUE_DEPTH: usize = 64;
/// `nice` value for the dropper thread (Linux): below consensus work.
const DROPPER_NICE: i32 = 10;

/// A value to drop plus a label for the log line.
pub struct DropJob {
    pub label: &'static str,
    pub items: usize,
    pub value: Box<dyn Send>,
    /// Purge jemalloc arenas after the drop.
    pub purge: bool,
}

/// Outcome of [`BackgroundDropper::submit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitted {
    /// Queued; it will be dropped on the dropper thread.
    Queued,
    /// The queue was full or the thread is gone; dropped on the caller.
    DroppedInline,
}

pub struct BackgroundDropper {
    sender: SyncSender<DropJob>,
}

impl BackgroundDropper {
    pub fn spawn() -> std::io::Result<Self> {
        let (sender, receiver) = sync_channel(QUEUE_DEPTH);
        std::thread::Builder::new()
            .name(DROPPER_THREAD_NAME.to_owned())
            .spawn(move || run(receiver))?;
        Ok(Self { sender })
    }

    /// The process-wide dropper, started on first use.
    pub fn global() -> &'static Self {
        static DROPPER: OnceLock<BackgroundDropper> = OnceLock::new();
        DROPPER.get_or_init(|| Self::spawn().expect("spawn background dropper thread"))
    }

    pub fn submit(&self, job: DropJob) -> Submitted {
        match self.sender.try_send(job) {
            Ok(()) => Submitted::Queued,
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                tracing::warn!(
                    target: "ledger",
                    event = "background_drop_inline",
                    label = job.label,
                    items = job.items,
                    "background dropper queue full; dropping on the caller"
                );
                drop(job);
                Submitted::DroppedInline
            }
        }
    }
}

fn lower_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        // PRIO_PROCESS with the thread id sets this thread's nice value only.
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        let _ = libc::setpriority(libc::PRIO_PROCESS, tid, DROPPER_NICE);
    }
}

fn run(receiver: Receiver<DropJob>) {
    lower_priority();
    while let Ok(job) = receiver.recv() {
        let DropJob {
            label,
            items,
            value,
            purge,
        } = job;
        let start = Instant::now();
        drop(value);
        let drop_ms = start.elapsed().as_millis() as u64;
        let purge_report = purge.then(|| {
            basics::memory::malloc_trim::malloc_trim(
                label,
                &basics::memory::malloc_trim::NullMallocTrimLogger,
            )
        });
        tracing::info!(
            target: "ledger",
            event = "background_drop_done",
            label,
            items,
            drop_ms,
            purge_us = purge_report.as_ref().map_or(0, |r| r.duration_us),
            rss_freed_kb = purge_report.as_ref().map_or(0, |r| -r.delta_kb()),
            "dropped detached values off the consensus thread"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    /// Records which thread dropped it, and is slow to drop.
    struct SlowDrop {
        dropped_on: Arc<Mutex<Option<String>>>,
        done: mpsc::Sender<()>,
    }
    impl Drop for SlowDrop {
        fn drop(&mut self) {
            std::thread::sleep(Duration::from_millis(300));
            *self.dropped_on.lock().unwrap() =
                std::thread::current().name().map(str::to_owned);
            let _ = self.done.send(());
        }
    }

    /// Regression for the Nov 12 02:46 stall: the submitter must not pay for
    /// the drop, and the drop must happen on the dropper thread.
    #[test]
    fn submit_returns_before_the_value_is_dropped_on_the_dropper_thread() {
        let dropper = BackgroundDropper::spawn().expect("spawn");
        let dropped_on = Arc::new(Mutex::new(None));
        let (done, wait) = mpsc::channel();
        let start = Instant::now();
        let outcome = dropper.submit(DropJob {
            label: "test",
            items: 1,
            value: Box::new(SlowDrop {
                dropped_on: Arc::clone(&dropped_on),
                done,
            }),
            purge: false,
        });
        let submit_time = start.elapsed();
        assert_eq!(outcome, Submitted::Queued);
        assert!(submit_time < Duration::from_millis(100), "{submit_time:?}");
        wait.recv_timeout(Duration::from_secs(5)).expect("dropped");
        assert_eq!(dropped_on.lock().unwrap().as_deref(), Some(DROPPER_THREAD_NAME));
    }

    #[test]
    fn full_queue_falls_back_to_dropping_inline() {
        let (sender, _receiver) = sync_channel(0);
        let dropper = BackgroundDropper { sender };
        let outcome = dropper.submit(DropJob {
            label: "test",
            items: 0,
            value: Box::new(()),
            purge: false,
        });
        assert_eq!(outcome, Submitted::DroppedInline);
    }
}
