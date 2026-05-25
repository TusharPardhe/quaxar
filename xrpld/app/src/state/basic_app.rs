//! Tokio-backed `BasicApp` shell.
//!
//! The reference `BasicApp` owns the I/O runtime threads. This Rust version keeps the
//! same lifecycle intent but makes the runtime explicit and easy to wire into
//! the rest of the app shell.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::runtime::{Builder, Handle, Runtime};

#[derive(Clone)]
pub struct BasicApp {
    runtime: Arc<Runtime>,
    worker_threads: usize,
}

impl std::fmt::Debug for BasicApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicApp")
            .field("worker_threads", &self.worker_threads)
            .finish()
    }
}

impl BasicApp {
    pub fn new(worker_threads: usize) -> io::Result<Self> {
        let runtime = if worker_threads == 0 {
            Builder::new_current_thread().enable_all().build()?
        } else {
            let names = Arc::new(AtomicUsize::new(0));
            Builder::new_multi_thread()
                .worker_threads(worker_threads)
                .thread_name_fn(move || {
                    let index = names.fetch_add(1, Ordering::Relaxed);
                    format!("io svc #{index}")
                })
                .enable_all()
                .build()?
        };

        Ok(Self {
            runtime: Arc::new(runtime),
            worker_threads,
        })
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn handle(&self) -> Handle {
        self.runtime.handle().clone()
    }

    pub fn worker_threads(&self) -> usize {
        self.worker_threads
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        self.runtime.block_on(future)
    }

    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }
}

#[cfg(test)]
mod tests {
    use super::BasicApp;
    use std::sync::{Arc, Mutex};

    #[test]
    fn basic_app_runs_work_on_the_runtime_shell() {
        let app = BasicApp::new(0).expect("runtime should build");
        assert_eq!(app.worker_threads(), 0);

        let result = app.block_on(async { 4usize + 5usize });
        assert_eq!(result, 9);
    }

    #[test]
    fn basic_app_can_spawn_tasks_through_the_runtime_handle() {
        let app = BasicApp::new(1).expect("runtime should build");
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let join = app.spawn(async move {
            events_clone
                .lock()
                .expect("events mutex must not be poisoned")
                .push("ran");
            11usize
        });

        let output = app.block_on(async { join.await.expect("spawned task should complete") });
        assert_eq!(output, 11);
        assert_eq!(
            events
                .lock()
                .expect("events mutex must not be poisoned")
                .as_slice(),
            &["ran"]
        );
    }
}
