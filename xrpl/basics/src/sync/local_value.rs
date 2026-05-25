//! Rust port of `xrpl/basics/LocalValue.h`.
//!
//! This port keeps the public `LocalValue<T>` surface thread-local by default,
//! while internal runtime owners can install hidden slot owners that model the

use crate::mutex::{RecursiveMutex, RecursiveMutexGuard};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

type SlotValue = Arc<dyn Any + Send + Sync>;
type SlotMap = HashMap<usize, SlotValue>;

#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct LocalSlotOwner {
    slots: Arc<Mutex<SlotMap>>,
}

impl LocalSlotOwner {
    #[doc(hidden)]
    pub fn new() -> Self {
        Self::default()
    }
}

thread_local! {
    static ACTIVE_LOCAL_CONTEXT: RefCell<Option<LocalSlotOwner>> = const { RefCell::new(None) };
    static THREAD_LOCAL_CONTEXT: LocalSlotOwner = LocalSlotOwner::new();
}

static NEXT_LOCAL_VALUE_ID: AtomicUsize = AtomicUsize::new(1);

/// Explicit local storage context for internal runtime tests and helpers.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Default)]
pub(crate) struct LocalContext {
    owner: LocalSlotOwner,
}

impl LocalContext {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Enter this context and restore the previous active context when dropped.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn enter(&self) -> LocalContextGuard {
        enter_local_context(self)
    }
}

/// Restores the previously active local context when dropped.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
#[must_use]
pub struct LocalContextGuard {
    previous: Option<LocalSlotOwner>,
    // Keep the guard tied to the thread that installed it.
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for LocalContextGuard {
    fn drop(&mut self) {
        ACTIVE_LOCAL_CONTEXT.with(|active| {
            *active.borrow_mut() = self.previous.take();
        });
    }
}

/// Enter a scoped local-storage context.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn enter_local_context(context: &LocalContext) -> LocalContextGuard {
    install_local_slot_owner(&context.owner)
}

/// Capture the currently effective slot owner so callers can later reinstall
/// the same coroutine/TSS-local state on this thread or another thread.
#[doc(hidden)]
pub fn capture_local_slot_owner() -> LocalSlotOwner {
    ACTIVE_LOCAL_CONTEXT.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .unwrap_or_else(|| THREAD_LOCAL_CONTEXT.with(LocalSlotOwner::clone))
    })
}

#[doc(hidden)]
pub fn install_local_slot_owner(owner: &LocalSlotOwner) -> LocalContextGuard {
    ACTIVE_LOCAL_CONTEXT.with(|active| {
        let previous = active.borrow_mut().replace(owner.clone());
        LocalContextGuard {
            previous,
            _not_send: PhantomData,
        }
    })
}

/// Stores a value that is local to either the current hidden runtime owner or
/// the current thread's default context.
#[derive(Debug)]
pub struct LocalValue<T> {
    id: usize,
    default: T,
}

/// Borrowed access to a scoped local value.
#[must_use = "LocalValueRef holds the active scoped borrow until it is dropped"]
pub struct LocalValueRef<T: 'static> {
    _value: RecursiveMutexGuard<'static, T>,
    keepalive: Arc<RecursiveMutex<T>>,
    value: *const T,
}

/// Mutable borrowed access to a scoped local value.
#[must_use = "LocalValueRefMut holds the active scoped borrow until it is dropped"]
pub struct LocalValueRefMut<T: 'static> {
    _value: RecursiveMutexGuard<'static, T>,
    keepalive: Arc<RecursiveMutex<T>>,
    value: *mut T,
}

impl<T> LocalValue<T>
where
    T: Clone + Send + 'static,
{
    pub fn new(default: T) -> Self {
        Self::new_with(|| default)
    }

    /// Construct the initial value lazily, mirroring the reference forwarding
    /// constructor without requiring callers to materialize the seed upfront.
    pub fn new_with(default: impl FnOnce() -> T) -> Self {
        Self {
            id: NEXT_LOCAL_VALUE_ID.fetch_add(1, Ordering::Relaxed),
            default: default(),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let value = self.borrow();
        f(&value)
    }

    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut value = self.borrow_mut();
        f(&mut value)
    }

    pub fn get_cloned(&self) -> T {
        self.with(Clone::clone)
    }

    pub fn set(&self, value: T) {
        self.with_mut(|slot| *slot = value);
    }

    pub fn replace(&self, value: T) -> T {
        self.with_mut(|slot| std::mem::replace(slot, value))
    }

    #[must_use = "borrow() returns a scoped guard that must be held while using the value"]
    pub fn borrow(&self) -> LocalValueRef<T> {
        let keepalive = self.current_slot();
        let value = keepalive
            .lock()
            .expect("LocalValue recursive mutex must not be poisoned");
        let slot = (&*value) as *const T;
        let value = unsafe {
            std::mem::transmute::<RecursiveMutexGuard<'_, T>, RecursiveMutexGuard<'static, T>>(
                value,
            )
        };
        LocalValueRef {
            _value: value,
            keepalive,
            value: slot,
        }
    }

    #[must_use = "borrow_mut() returns a scoped guard that must be held while using the value"]
    pub fn borrow_mut(&self) -> LocalValueRefMut<T> {
        let keepalive = self.current_slot();
        let mut value = keepalive
            .lock()
            .expect("LocalValue recursive mutex must not be poisoned");
        let slot = (&mut *value) as *mut T;
        let value = unsafe {
            std::mem::transmute::<RecursiveMutexGuard<'_, T>, RecursiveMutexGuard<'static, T>>(
                value,
            )
        };
        LocalValueRefMut {
            _value: value,
            keepalive,
            value: slot,
        }
    }

    fn current_slot(&self) -> Arc<RecursiveMutex<T>> {
        let keepalive = current_slots();
        let slot = {
            let mut slots = keepalive
                .lock()
                .expect("LocalValue mutex must not be poisoned");
            slots
                .entry(self.id)
                .or_insert_with(|| Arc::new(RecursiveMutex::new(self.default.clone())))
                .clone()
        };

        Arc::downcast::<RecursiveMutex<T>>(slot)
            .expect("LocalValue slot type must match the LocalValue instance")
    }
}

impl<T> Default for LocalValue<T>
where
    T: Clone + Default + Send + 'static,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

fn current_slots() -> Arc<Mutex<SlotMap>> {
    ACTIVE_LOCAL_CONTEXT.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(|owner| Arc::clone(&owner.slots))
            .unwrap_or_else(|| THREAD_LOCAL_CONTEXT.with(|owner| Arc::clone(&owner.slots)))
    })
}

impl<T> Deref for LocalValueRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let _keep_alive = &self.keepalive;
        // SAFETY: `value` points into the map protected by `slots`, which this
        // guard keeps locked for the lifetime of the reference.
        unsafe { &*self.value }
    }
}

impl<T> Deref for LocalValueRefMut<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let _keep_alive = &self.keepalive;
        // SAFETY: `value` points into the map protected by `slots`, which this
        // guard keeps locked for the lifetime of the reference.
        unsafe { &*self.value }
    }
}

impl<T> DerefMut for LocalValueRefMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let _keep_alive = &self.keepalive;
        // SAFETY: `value` points into the map protected by `slots`, which this
        // guard keeps locked for the lifetime of the mutable reference.
        unsafe { &mut *self.value }
    }
}

impl<T> AsRef<T> for LocalValueRef<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsRef<T> for LocalValueRefMut<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for LocalValueRefMut<T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LocalContext, LocalSlotOwner, LocalValue, capture_local_slot_owner, enter_local_context,
        install_local_slot_owner,
    };
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn default_values_are_thread_local_outside_contexts() {
        let value = Arc::new(LocalValue::new(-1));
        assert_eq!(value.get_cloned(), -1);

        let first = {
            let value = Arc::clone(&value);
            thread::spawn(move || {
                assert_eq!(value.get_cloned(), -1);
                value.set(-2);
                value.get_cloned()
            })
        }
        .join()
        .expect("thread should complete");

        assert_eq!(first, -2);
        assert_eq!(value.get_cloned(), -1);

        let second = {
            let value = Arc::clone(&value);
            thread::spawn(move || value.get_cloned())
        }
        .join()
        .expect("thread should complete");

        assert_eq!(second, -1);
    }

    #[test]
    fn context_reentry_preserves_values_like_coroutine_resume() {
        let value = Arc::new(LocalValue::new(-1));
        let context = Arc::new(LocalContext::new());

        {
            let _guard = enter_local_context(&context);
            assert_eq!(value.get_cloned(), -1);
            value.set(7);
            assert_eq!(value.get_cloned(), 7);
        }

        assert_eq!(value.get_cloned(), -1);

        {
            let _guard = enter_local_context(&context);
            assert_eq!(value.get_cloned(), 7);
        }

        let resumed_on_another_thread = {
            let value = Arc::clone(&value);
            let context = Arc::clone(&context);
            thread::spawn(move || {
                let _guard = enter_local_context(&context);
                assert_eq!(value.get_cloned(), 7);
                value.set(9);
                value.get_cloned()
            })
        }
        .join()
        .expect("thread should complete");

        assert_eq!(resumed_on_another_thread, 9);

        {
            let _guard = enter_local_context(&context);
            assert_eq!(value.get_cloned(), 9);
        }
    }

    #[test]
    fn nested_contexts_restore_previous_scope() {
        let value = LocalValue::new(0);
        let first = LocalContext::new();
        let second = LocalContext::new();

        {
            let _first = enter_local_context(&first);
            value.set(11);
            assert_eq!(value.get_cloned(), 11);

            {
                let _second = enter_local_context(&second);
                assert_eq!(value.get_cloned(), 0);
                value.set(22);
                assert_eq!(value.get_cloned(), 22);
            }

            assert_eq!(value.get_cloned(), 11);
        }
    }

    #[test]
    fn separate_local_values_do_not_share_slots() {
        let first = LocalValue::new(String::from("alpha"));
        let second = LocalValue::new(String::from("beta"));
        let context = LocalContext::new();
        let _guard = enter_local_context(&context);

        first.with_mut(|value| value.push_str("-ctx"));

        assert_eq!(first.get_cloned(), "alpha-ctx");
        assert_eq!(second.get_cloned(), "beta");
    }

    #[test]
    fn hidden_slot_owner_install_restores_previous_scope_like_coro_resume() {
        let value = LocalValue::new(-1);
        let explicit = LocalContext::new();
        let hidden = LocalSlotOwner::new();

        {
            let _outer = explicit.enter();
            value.set(7);
            assert_eq!(value.get_cloned(), 7);

            {
                let _hidden = install_local_slot_owner(&hidden);
                assert_eq!(value.get_cloned(), -1);
                value.set(11);
                assert_eq!(value.get_cloned(), 11);
            }

            assert_eq!(value.get_cloned(), 7);
        }

        assert_eq!(value.get_cloned(), -1);
    }

    #[test]
    fn captured_thread_local_owner_restores_previous_thread_state_like_tss_handoff() {
        let value = LocalValue::new(-1);
        value.set(-2);

        let thread_owner = capture_local_slot_owner();
        let hidden = LocalSlotOwner::new();

        {
            let _hidden = install_local_slot_owner(&hidden);
            assert_eq!(value.get_cloned(), -1);
            value.set(11);
            assert_eq!(value.get_cloned(), 11);
        }

        assert_eq!(value.get_cloned(), -2);

        {
            let _restore = install_local_slot_owner(&thread_owner);
            assert_eq!(value.get_cloned(), -2);
        }

        assert_eq!(value.get_cloned(), -2);
    }

    #[test]
    fn captured_hidden_owner_reinstalls_values_on_another_thread_like_coro_resume() {
        let value = Arc::new(LocalValue::new(-1));
        let hidden = LocalSlotOwner::new();

        let captured = {
            let _hidden = install_local_slot_owner(&hidden);
            value.set(23);
            assert_eq!(value.get_cloned(), 23);
            capture_local_slot_owner()
        };

        assert_eq!(value.get_cloned(), -1);

        let resumed = {
            let value = Arc::clone(&value);
            thread::spawn(move || {
                assert_eq!(value.get_cloned(), -1);
                let _restore = install_local_slot_owner(&captured);
                assert_eq!(value.get_cloned(), 23);
                value.set(29);
                value.get_cloned()
            })
        }
        .join()
        .expect("thread should complete");

        assert_eq!(resumed, 29);

        {
            let _restore = install_local_slot_owner(&hidden);
            assert_eq!(value.get_cloned(), 29);
        }

        assert_eq!(value.get_cloned(), -1);
    }
}
