//! Compatibility wrapper for `xrpl/basics/Mutex.hpp`.
//!
//! The reference surface is generic over both the protected data type and the mutex
//! backend, and it exposes the same access shape through const and mutable
//! locks. Rust cannot overload `operator*`/`operator->`, so this wrapper keeps
//! the same idea with explicit `lock`, `lock_shared`, `lock_with`, `get`, and
//! `get_mut` methods.

use std::cell::UnsafeCell;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{
    LockResult, Mutex as StdMutex, MutexGuard as StdMutexGuard, RwLock as StdRwLock,
    RwLockReadGuard, RwLockWriteGuard, TryLockError, TryLockResult,
};

pub trait MutexBackend<T> {
    type SharedGuard<'a>: Deref<Target = T>
    where
        Self: 'a,
        T: 'a;

    type ExclusiveGuard<'a>: Deref<Target = T> + DerefMut<Target = T>
    where
        Self: 'a,
        T: 'a;

    fn new(data: T) -> Self
    where
        Self: Sized;

    fn lock_shared(&self) -> Self::SharedGuard<'_>;

    fn lock_exclusive(&self) -> Self::ExclusiveGuard<'_>;
}

pub trait UnlockableMutexBackend<T>: MutexBackend<T> {
    type UniqueGuard<'a>: Deref<Target = T> + DerefMut<Target = T>
    where
        Self: 'a,
        T: 'a;

    fn unique_lock(&self) -> Self::UniqueGuard<'_>;

    fn try_unique_lock(&self) -> TryLockResult<Self::UniqueGuard<'_>>;
}

pub trait MutexLockMode<T, Backend>
where
    Backend: MutexBackend<T>,
{
    type Guard<'a>: Deref<Target = T>
    where
        Self: 'a,
        Backend: 'a,
        T: 'a;

    fn lock(backend: &Backend) -> Self::Guard<'_>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExclusiveLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SharedLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UniqueLock;

/// Recursive mutex matching reference std::recursive_mutex.
/// Backed by parking_lot::ReentrantMutex for fast lock/unlock
/// (no thread::current().id() call, no inner Mutex, spin-then-park).
pub struct RecursiveMutex<T> {
    inner: parking_lot::ReentrantMutex<UnsafeCell<T>>,
}

pub struct RecursiveMutexGuard<'a, T> {
    guard: parking_lot::ReentrantMutexGuard<'a, UnsafeCell<T>>,
}

pub struct RecursiveMutexUniqueLock<'a, T> {
    guard: Option<parking_lot::ReentrantMutexGuard<'a, UnsafeCell<T>>>,
    mutex: &'a RecursiveMutex<T>,
}

impl<T> RecursiveMutex<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: parking_lot::ReentrantMutex::new(UnsafeCell::new(value)),
        }
    }

    pub fn lock(&self) -> LockResult<RecursiveMutexGuard<'_, T>> {
        Ok(RecursiveMutexGuard {
            guard: self.inner.lock(),
        })
    }

    pub fn try_lock(&self) -> TryLockResult<RecursiveMutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Some(guard) => Ok(RecursiveMutexGuard { guard }),
            None => Err(TryLockError::WouldBlock),
        }
    }

    pub fn unique_lock(&self) -> LockResult<RecursiveMutexUniqueLock<'_, T>> {
        let guard = self.inner.lock();
        Ok(RecursiveMutexUniqueLock {
            guard: Some(guard),
            mutex: self,
        })
    }

    pub fn try_unique_lock(&self) -> TryLockResult<RecursiveMutexUniqueLock<'_, T>> {
        match self.inner.try_lock() {
            Some(guard) => Ok(RecursiveMutexUniqueLock {
                guard: Some(guard),
                mutex: self,
            }),
            None => Err(TryLockError::WouldBlock),
        }
    }
}

impl<T> fmt::Debug for RecursiveMutex<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecursiveMutex").finish_non_exhaustive()
    }
}

impl<T> Deref for RecursiveMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.guard.get() }
    }
}

impl<T> DerefMut for RecursiveMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.guard.get() }
    }
}

// Drop is automatic — parking_lot guard releases on drop

impl<T> RecursiveMutexUniqueLock<'_, T> {
    pub fn is_locked(&self) -> bool {
        self.guard.is_some()
    }

    pub fn unlock(&mut self) {
        assert!(
            self.guard.is_some(),
            "recursive unique lock is already unlocked"
        );
        self.guard = None;
    }

    pub fn lock(&mut self) -> LockResult<()> {
        if self.guard.is_none() {
            self.guard = Some(self.mutex.inner.lock());
        }
        Ok(())
    }

    pub fn try_lock(&mut self) -> TryLockResult<()> {
        if self.guard.is_some() {
            return Ok(());
        }
        match self.mutex.inner.try_lock() {
            Some(g) => {
                self.guard = Some(g);
                Ok(())
            }
            None => Err(TryLockError::WouldBlock),
        }
    }
}

impl<T> Deref for RecursiveMutexUniqueLock<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        let guard = self.guard.as_ref().expect("unique lock must be locked");
        unsafe { &*guard.get() }
    }
}

impl<T> DerefMut for RecursiveMutexUniqueLock<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let guard = self.guard.as_ref().expect("unique lock must be locked");
        unsafe { &mut *guard.get() }
    }
}

// Drop is automatic — Option<Guard> drops the guard if Some

unsafe impl<T: Send> Send for RecursiveMutex<T> {}
unsafe impl<T: Send> Sync for RecursiveMutex<T> {}

impl<T> MutexBackend<T> for StdMutex<T> {
    type SharedGuard<'a>
        = StdMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;
    type ExclusiveGuard<'a>
        = StdMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(data: T) -> Self {
        StdMutex::new(data)
    }

    fn lock_shared(&self) -> Self::SharedGuard<'_> {
        self.lock().expect("mutex must not be poisoned")
    }

    fn lock_exclusive(&self) -> Self::ExclusiveGuard<'_> {
        self.lock().expect("mutex must not be poisoned")
    }
}

impl<T> MutexBackend<T> for StdRwLock<T> {
    type SharedGuard<'a>
        = RwLockReadGuard<'a, T>
    where
        Self: 'a,
        T: 'a;
    type ExclusiveGuard<'a>
        = RwLockWriteGuard<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(data: T) -> Self {
        StdRwLock::new(data)
    }

    fn lock_shared(&self) -> Self::SharedGuard<'_> {
        self.read().expect("rwlock must not be poisoned")
    }

    fn lock_exclusive(&self) -> Self::ExclusiveGuard<'_> {
        self.write().expect("rwlock must not be poisoned")
    }
}

impl<T> MutexBackend<T> for RecursiveMutex<T> {
    type SharedGuard<'a>
        = RecursiveMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;
    type ExclusiveGuard<'a>
        = RecursiveMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(data: T) -> Self {
        RecursiveMutex::new(data)
    }

    fn lock_shared(&self) -> Self::SharedGuard<'_> {
        self.lock().expect("recursive mutex must not be poisoned")
    }

    fn lock_exclusive(&self) -> Self::ExclusiveGuard<'_> {
        self.lock().expect("recursive mutex must not be poisoned")
    }
}

impl<T> UnlockableMutexBackend<T> for RecursiveMutex<T> {
    type UniqueGuard<'a>
        = RecursiveMutexUniqueLock<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn unique_lock(&self) -> Self::UniqueGuard<'_> {
        RecursiveMutex::unique_lock(self).expect("recursive mutex unique lock must not be poisoned")
    }

    fn try_unique_lock(&self) -> TryLockResult<Self::UniqueGuard<'_>> {
        RecursiveMutex::try_unique_lock(self)
    }
}

impl<T, Backend> MutexLockMode<T, Backend> for ExclusiveLock
where
    Backend: MutexBackend<T>,
{
    type Guard<'a>
        = Backend::ExclusiveGuard<'a>
    where
        Self: 'a,
        Backend: 'a,
        T: 'a;

    fn lock(backend: &Backend) -> Self::Guard<'_> {
        backend.lock_exclusive()
    }
}

impl<T, Backend> MutexLockMode<T, Backend> for SharedLock
where
    Backend: MutexBackend<T>,
{
    type Guard<'a>
        = Backend::SharedGuard<'a>
    where
        Self: 'a,
        Backend: 'a,
        T: 'a;

    fn lock(backend: &Backend) -> Self::Guard<'_> {
        backend.lock_shared()
    }
}

impl<T, Backend> MutexLockMode<T, Backend> for UniqueLock
where
    Backend: UnlockableMutexBackend<T>,
{
    type Guard<'a>
        = Backend::UniqueGuard<'a>
    where
        Self: 'a,
        Backend: 'a,
        T: 'a;

    fn lock(backend: &Backend) -> Self::Guard<'_> {
        backend.unique_lock()
    }
}

pub struct Mutex<T, Backend = StdMutex<T>>
where
    Backend: MutexBackend<T>,
{
    inner: Backend,
    _marker: PhantomData<fn() -> T>,
}

impl<T, Backend> fmt::Debug for Mutex<T, Backend>
where
    Backend: MutexBackend<T> + fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mutex").field("inner", &self.inner).finish()
    }
}

impl<T, Backend> Mutex<T, Backend>
where
    Backend: MutexBackend<T>,
{
    pub fn new(data: T) -> Self {
        Self {
            inner: Backend::new(data),
            _marker: PhantomData,
        }
    }

    pub fn make() -> Self
    where
        T: Default,
    {
        Self::new(T::default())
    }

    pub fn make_with<F>(factory: F) -> Self
    where
        F: FnOnce() -> T,
    {
        Self::new(factory())
    }

    pub fn make_from(value: T) -> Self {
        Self::new(value)
    }

    pub fn lock_with<Mode>(&self) -> Lock<'_, T, Mode::Guard<'_>>
    where
        Mode: MutexLockMode<T, Backend>,
    {
        Lock::new(Mode::lock(&self.inner))
    }

    pub fn lock(&self) -> Lock<'_, T, Backend::ExclusiveGuard<'_>> {
        self.lock_with::<ExclusiveLock>()
    }

    pub fn lock_shared(&self) -> Lock<'_, T, Backend::SharedGuard<'_>> {
        self.lock_with::<SharedLock>()
    }
}

impl<T, Backend> Default for Mutex<T, Backend>
where
    Backend: MutexBackend<T>,
    T: Default,
{
    fn default() -> Self {
        Self::make()
    }
}

pub struct Lock<'a, T, Guard>
where
    Guard: Deref<Target = T>,
{
    guard: Guard,
    _marker: PhantomData<&'a T>,
}

impl<'a, T, Guard> Lock<'a, T, Guard>
where
    Guard: Deref<Target = T>,
{
    fn new(guard: Guard) -> Self {
        Self {
            guard,
            _marker: PhantomData,
        }
    }

    pub fn get(&self) -> &T {
        &self.guard
    }

    pub fn into_guard(self) -> Guard {
        self.guard
    }

    pub fn guard(&self) -> &Guard {
        &self.guard
    }
}

impl<'a, T, Guard> Lock<'a, T, Guard>
where
    Guard: Deref<Target = T> + DerefMut<Target = T>,
{
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.guard
    }

    pub fn guard_mut(&mut self) -> &mut Guard {
        &mut self.guard
    }
}

impl<'a, T, Guard> Deref for Lock<'a, T, Guard>
where
    Guard: Deref<Target = T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<'a, T, Guard> DerefMut for Lock<'a, T, Guard>
where
    Guard: Deref<Target = T> + DerefMut<Target = T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::{Mutex, RecursiveMutex, StdRwLock};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    #[test]
    fn make_and_default_match_cpp_construction_shape() {
        let default_mutex = Mutex::<i32>::make();
        assert_eq!(*default_mutex.lock(), 0);

        let explicit_default = Mutex::<String>::default();
        assert_eq!(explicit_default.lock().get(), "");

        let string_mutex = Mutex::<String>::make_from("test".to_owned());
        assert_eq!(string_mutex.lock().get(), "test");
    }

    #[test]
    fn make_with_supports_multi_argument_construction_via_closure() {
        #[derive(Debug, PartialEq, Eq)]
        struct Data {
            x: i32,
            y: String,
        }

        let mutex = Mutex::<Data>::make_with(|| Data {
            x: 42,
            y: "hello".to_owned(),
        });
        let lock = mutex.lock();
        assert_eq!(lock.get().x, 42);
        assert_eq!(lock.get().y, "hello");
    }

    #[test]
    fn move_only_and_mutable_access_match_cpp_roles() {
        let mutex = Mutex::<Box<i32>>::make_from(Box::new(100));
        {
            let mut lock = mutex.lock();
            assert_eq!(**lock, 100);
            *lock.get_mut() = Box::new(200);
        }
        assert_eq!(**mutex.lock(), 200);
    }

    #[test]
    fn generic_shared_lock_on_rwlock_backend_provides_const_access() {
        let mutex = Mutex::<i32, StdRwLock<i32>>::new(100);
        {
            let lock = mutex.lock_shared();
            assert_eq!(*lock, 100);
        }

        {
            let mut lock = mutex.lock();
            *lock = 200;
        }

        let lock = mutex.lock_shared();
        assert_eq!(*lock, 200);
    }

    #[test]
    fn lock_is_thread_safe_for_sequential_access() {
        let mutex = Arc::new(Mutex::<Vec<i32>>::new(vec![1, 2]));
        let other = Arc::clone(&mutex);

        let handle = thread::spawn(move || {
            let mut lock = other.lock();
            lock.push(3);
            lock.len()
        });

        assert_eq!(handle.join().expect("thread"), 3);
        assert_eq!(mutex.lock().get(), &vec![1, 2, 3]);
    }

    #[test]
    fn recursive_mutex_backend_allows_same_thread_reentry() {
        let mutex = Mutex::<i32, RecursiveMutex<i32>>::new(7);
        let mut outer = mutex.lock();
        assert_eq!(*outer, 7);

        {
            let inner = mutex.lock();
            assert_eq!(*inner, 7);
        }

        *outer.get_mut() = 9;
        assert_eq!(*mutex.lock(), 9);
    }

    #[test]
    fn recursive_mutex_try_lock_blocks_other_threads_while_owned() {
        let mutex = Arc::new(RecursiveMutex::new(1));
        let _guard = mutex.lock().expect("lock");
        let blocked = Arc::new(AtomicBool::new(false));

        let join = thread::spawn({
            let mutex = Arc::clone(&mutex);
            let blocked = Arc::clone(&blocked);
            move || {
                blocked.store(mutex.try_lock().is_err(), Ordering::SeqCst);
            }
        });

        join.join().expect("thread join");
        assert!(blocked.load(Ordering::SeqCst));
    }
}
