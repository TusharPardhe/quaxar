//! LegacyPathFind rate limiter ported from `xrpld/rpc/detail/LegacyPathFind.h/the reference source`.
//!
//! Limits concurrent path_find operations using an atomic counter.
//! Admin users bypass the limit. Non-admin users are rejected if:
//! - Job queue client count exceeds `MAX_PATHFIND_JOB_COUNT`
//! - The server is locally overloaded
//! - The number of in-progress pathfinds is at `MAX_PATHFINDS_IN_PROGRESS`

#![allow(dead_code)]

use std::sync::atomic::{AtomicI32, Ordering};

use crate::state::tuning::Tuning;

/// Global atomic counter for in-progress pathfind operations.
static IN_PROGRESS: AtomicI32 = AtomicI32::new(0);

/// Trait representing the application context needed for rate limiting.
pub trait PathFindApp {
    /// Returns the number of jobs at or above the client priority level.
    fn job_count_ge_client(&self) -> u32;
    /// Returns true if the server is locally overloaded.
    fn is_loaded_local(&self) -> bool;
}

/// RAII guard that limits concurrent pathfind operations.
///
/// On construction, attempts to acquire a slot. If successful, `is_ok()` returns true.
/// On drop, releases the slot if it was acquired.
///
/// Admin users always succeed. Non-admin users are subject to:
/// - Job queue count check
/// - Load check
/// - Atomic CAS loop for the concurrency limit
pub struct LegacyPathFind {
    is_ok: bool,
}

impl LegacyPathFind {
    /// Attempt to acquire a pathfind slot.
    ///
    /// `is_admin`: admin users bypass all checks.
    /// `app`: application context for job queue and load checks.
    pub fn new(is_admin: bool, app: &dyn PathFindApp) -> Self {
        if is_admin {
            IN_PROGRESS.fetch_add(1, Ordering::Release);
            return Self { is_ok: true };
        }

        // Check job queue and load
        if app.job_count_ge_client() > Tuning::MAX_PATHFIND_JOB_COUNT || app.is_loaded_local() {
            return Self { is_ok: false };
        }

        // CAS loop to atomically increment if below limit
        loop {
            let prev = IN_PROGRESS.load(Ordering::Relaxed);
            if prev >= Tuning::MAX_PATHFINDS_IN_PROGRESS as i32 {
                return Self { is_ok: false };
            }

            if IN_PROGRESS
                .compare_exchange_weak(prev, prev + 1, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return Self { is_ok: true };
            }
        }
    }

    /// Returns true if the pathfind slot was successfully acquired.
    pub fn is_ok(&self) -> bool {
        self.is_ok
    }

    /// Returns the current number of in-progress pathfinds (for diagnostics).
    pub fn current_count() -> i32 {
        IN_PROGRESS.load(Ordering::Relaxed)
    }
}

impl Drop for LegacyPathFind {
    fn drop(&mut self) {
        if self.is_ok {
            IN_PROGRESS.fetch_sub(1, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockApp {
        job_count: u32,
        loaded: bool,
    }

    impl PathFindApp for MockApp {
        fn job_count_ge_client(&self) -> u32 {
            self.job_count
        }
        fn is_loaded_local(&self) -> bool {
            self.loaded
        }
    }

    #[test]
    fn admin_always_succeeds() {
        let app = MockApp {
            job_count: 1000,
            loaded: true,
        };
        let pf = LegacyPathFind::new(true, &app);
        assert!(pf.is_ok());
    }

    #[test]
    fn rejects_when_loaded() {
        let app = MockApp {
            job_count: 0,
            loaded: true,
        };
        let pf = LegacyPathFind::new(false, &app);
        assert!(!pf.is_ok());
    }

    #[test]
    fn rejects_when_too_many_jobs() {
        let app = MockApp {
            job_count: Tuning::MAX_PATHFIND_JOB_COUNT + 1,
            loaded: false,
        };
        let pf = LegacyPathFind::new(false, &app);
        assert!(!pf.is_ok());
    }

    #[test]
    fn respects_concurrency_limit() {
        let app = MockApp {
            job_count: 0,
            loaded: false,
        };

        // Acquire up to the limit
        let guards: Vec<_> = (0..Tuning::MAX_PATHFINDS_IN_PROGRESS)
            .map(|_| LegacyPathFind::new(false, &app))
            .collect();
        assert!(guards.iter().all(|g| g.is_ok()));

        // Next one should fail
        let extra = LegacyPathFind::new(false, &app);
        assert!(!extra.is_ok());

        // Drop guards and try again
        drop(guards);
        let retry = LegacyPathFind::new(false, &app);
        assert!(retry.is_ok());
    }
}
