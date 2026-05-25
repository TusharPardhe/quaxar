use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosureCounterState {
    pub count: i32,
    pub joined: bool,
}

#[derive(Debug)]
struct ClosureCounterInner {
    wait_for_closures: Mutex<bool>,
    all_closures_done: Condvar,
    closure_count: AtomicI32,
}

impl ClosureCounterInner {
    fn increment(&self) {
        self.closure_count.fetch_add(1, Ordering::SeqCst);
    }

    fn decrement(&self) {
        let wait_for = self
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned");
        let next = self.closure_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if next == 0 && *wait_for {
            self.all_closures_done.notify_all();
        }
        drop(wait_for);
    }
}

pub struct CountedClosure<F> {
    inner: Arc<ClosureCounterInner>,
    closure: F,
}

impl<F> CountedClosure<F> {
    fn new(inner: Arc<ClosureCounterInner>, closure: F) -> Self {
        inner.increment();
        Self { inner, closure }
    }

    fn from_counted(inner: Arc<ClosureCounterInner>, closure: F) -> Self {
        Self { inner, closure }
    }
}

impl<F: Clone> Clone for CountedClosure<F> {
    fn clone(&self) -> Self {
        Self::new(Arc::clone(&self.inner), self.closure.clone())
    }
}

impl<F> Drop for CountedClosure<F> {
    fn drop(&mut self) {
        self.inner.decrement();
    }
}

impl<F> Deref for CountedClosure<F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        &self.closure
    }
}

impl<F> DerefMut for CountedClosure<F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.closure
    }
}

pub struct ClosureCounter<Signature> {
    inner: Arc<ClosureCounterInner>,
    _marker: PhantomData<Signature>,
}

impl<Signature> Default for ClosureCounter<Signature> {
    fn default() -> Self {
        Self {
            inner: Arc::new(ClosureCounterInner {
                wait_for_closures: Mutex::new(false),
                all_closures_done: Condvar::new(),
                closure_count: AtomicI32::new(0),
            }),
            _marker: PhantomData,
        }
    }
}

impl<Signature> ClosureCounter<Signature> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> i32 {
        self.inner.closure_count.load(Ordering::SeqCst)
    }

    pub fn joined(&self) -> bool {
        *self
            .inner
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned")
    }

    pub fn state(&self) -> ClosureCounterState {
        ClosureCounterState {
            count: self.count(),
            joined: self.joined(),
        }
    }

    pub fn join<OnTimeout>(&self, _name: &str, wait: Duration, on_timeout: OnTimeout)
    where
        OnTimeout: FnOnce(),
    {
        let mut on_timeout = Some(on_timeout);
        let mut joined = self
            .inner
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned");
        *joined = true;

        if self.count() > 0 {
            let deadline = Instant::now() + wait;
            let mut logged = false;
            while self.count() > 0 {
                let now = Instant::now();
                if !logged && now >= deadline {
                    if let Some(on_timeout) = on_timeout.take() {
                        on_timeout();
                    }
                    logged = true;
                }
                let timeout = if logged {
                    Duration::from_millis(10)
                } else {
                    deadline.saturating_duration_since(now)
                };
                let (next_joined, _) = self
                    .inner
                    .all_closures_done
                    .wait_timeout(joined, timeout)
                    .expect("closure counter condvar wait must succeed");
                joined = next_joined;
            }
        }
    }
}

impl<Ret> ClosureCounter<fn() -> Ret> {
    pub fn wrap<F>(&self, closure: F) -> Option<CountedClosure<F>>
    where
        F: FnMut() -> Ret,
    {
        let joined = self
            .inner
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned");
        if *joined {
            None
        } else {
            self.inner.increment();
            drop(joined);
            Some(CountedClosure::from_counted(
                Arc::clone(&self.inner),
                closure,
            ))
        }
    }
}

impl<Ret, A> ClosureCounter<fn(A) -> Ret> {
    pub fn wrap<F>(&self, closure: F) -> Option<CountedClosure<F>>
    where
        F: FnMut(A) -> Ret,
    {
        let joined = self
            .inner
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned");
        if *joined {
            None
        } else {
            self.inner.increment();
            drop(joined);
            Some(CountedClosure::from_counted(
                Arc::clone(&self.inner),
                closure,
            ))
        }
    }
}

impl<Ret, A, B> ClosureCounter<fn(A, B) -> Ret> {
    pub fn wrap<F>(&self, closure: F) -> Option<CountedClosure<F>>
    where
        F: FnMut(A, B) -> Ret,
    {
        let joined = self
            .inner
            .wait_for_closures
            .lock()
            .expect("closure counter mutex poisoned");
        if *joined {
            None
        } else {
            self.inner.increment();
            drop(joined);
            Some(CountedClosure::from_counted(
                Arc::clone(&self.inner),
                closure,
            ))
        }
    }
}

impl<Signature> Drop for ClosureCounter<Signature> {
    fn drop(&mut self) {
        self.join("ClosureCounter", Duration::from_secs(1), || {});
    }
}
