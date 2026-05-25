//! Counting semaphore compatible with the current `xrpl/core/detail/semaphore.h`.
//!
//! This stays deliberately small: a mutex-protected counter plus a condition
//! variable. That matches the the reference implementation fallback used by `Workers` and keeps
//! the wakeup semantics deterministic for the worker pool tests.

use std::sync::{Condvar, Mutex};

#[derive(Debug)]
pub struct BasicSemaphore {
    count: Mutex<usize>,
    cond: Condvar,
}

impl BasicSemaphore {
    /// Create the semaphore with an optional initial count.
    pub fn new(count: usize) -> Self {
        Self {
            count: Mutex::new(count),
            cond: Condvar::new(),
        }
    }

    /// Increment the count and unblock one waiting thread.
    pub fn notify(&self) {
        let mut count = self.count.lock().expect("semaphore mutex poisoned");
        *count += 1;
        self.cond.notify_one();
    }

    /// Block until a token is available.
    pub fn wait(&self) {
        let mut count = self.count.lock().expect("semaphore mutex poisoned");
        while *count == 0 {
            count = self
                .cond
                .wait(count)
                .expect("semaphore condvar wait poisoned");
        }
        *count -= 1;
    }

    /// Non-blocking wait.
    pub fn try_wait(&self) -> bool {
        let mut count = self.count.lock().expect("semaphore mutex poisoned");
        if *count == 0 {
            false
        } else {
            *count -= 1;
            true
        }
    }
}

impl Default for BasicSemaphore {
    fn default() -> Self {
        Self::new(0)
    }
}

pub type Semaphore = BasicSemaphore;

#[cfg(test)]
mod tests {
    use super::BasicSemaphore;

    #[test]
    fn semaphore_notify_wait_and_try_wait_match_basic_cxx_shape() {
        let sem = BasicSemaphore::new(0);
        assert!(!sem.try_wait());

        sem.notify();
        assert!(sem.try_wait());
        assert!(!sem.try_wait());

        sem.notify();
        sem.wait();
        assert!(!sem.try_wait());
    }
}
