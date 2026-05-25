//! Rust port of `xrpl/basics/CountedObject.h`.
//!
//! The reference code uses inheritance to automatically count live instances of a
//! type and to expose named counters for reporting.
//!
//! Rust does not have inheritance, so this module offers:
//! - `CountedObject<T>` as a lifecycle-counting guard,
//! - `Counted<T>` as a wrapper that owns a value and counts it automatically,
//! - `Counter` for ad hoc named counters like `"CachedView::hit"`.

use std::any::type_name;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// A `(name, count)` entry returned by the global counted-object registry.
pub type Entry = (String, i32);

#[derive(Debug)]
struct CounterEntry {
    name: String,
    count: AtomicI32,
}

impl CounterEntry {
    fn new(name: String) -> Self {
        Self {
            name,
            count: AtomicI32::new(0),
        }
    }
}

/// Global registry of counted-object and named-counter entries.
#[derive(Debug, Default)]
pub struct CountedObjects {
    entries: Mutex<Vec<Arc<CounterEntry>>>,
    typed_registry: Mutex<HashMap<&'static str, Arc<CounterEntry>>>,
}

impl CountedObjects {
    pub fn get_instance() -> &'static Self {
        static INSTANCE: OnceLock<CountedObjects> = OnceLock::new();
        INSTANCE.get_or_init(Self::default)
    }

    pub fn get_counts(&self, minimum_threshold: i32) -> Vec<Entry> {
        let entries = self
            .entries
            .lock()
            .expect("CountedObjects registry mutex must not be poisoned");

        let mut counts = entries
            .iter()
            .filter_map(|entry| {
                let count = entry.count.load(Ordering::Relaxed);
                (count >= minimum_threshold).then(|| (entry.name.clone(), count))
            })
            .collect::<Vec<_>>();

        counts.sort();
        counts
    }

    fn counter_named(&self, name: impl Into<String>) -> Counter {
        let entry = Arc::new(CounterEntry::new(name.into()));
        self.entries
            .lock()
            .expect("CountedObjects registry mutex must not be poisoned")
            .push(entry.clone());

        Counter { entry }
    }

    fn counter_for_type<T>(&self) -> Counter {
        self.counter_for_type_named::<T>(short_type_name::<T>())
    }

    fn counter_for_type_named<T>(&self, display_name: impl Into<String>) -> Counter {
        let type_key = type_name::<T>();
        let display_name = display_name.into();
        let mut typed_registry = self
            .typed_registry
            .lock()
            .expect("CountedObjects typed registry mutex must not be poisoned");

        if let Some(entry) = typed_registry.get(type_key) {
            debug_assert_eq!(entry.name, display_name);
            return Counter {
                entry: entry.clone(),
            };
        }

        let entry = Arc::new(CounterEntry::new(display_name));
        typed_registry.insert(type_key, entry.clone());

        self.entries
            .lock()
            .expect("CountedObjects registry mutex must not be poisoned")
            .push(entry.clone());

        Counter { entry }
    }
}

/// Named counter used for ad hoc runtime metrics.
///
/// Each `Counter::new("name")` call creates a distinct registry entry. This
/// keeps ad hoc counters close to the reference shape, where each static counter is
/// its own object even if another counter happens to reuse the same label.
#[derive(Clone, Debug)]
pub struct Counter {
    entry: Arc<CounterEntry>,
}

impl Counter {
    pub fn new(name: impl Into<String>) -> Self {
        CountedObjects::get_instance().counter_named(name)
    }

    pub fn increment(&self) -> i32 {
        self.entry.count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn decrement(&self) -> i32 {
        self.entry.count.fetch_sub(1, Ordering::Relaxed) - 1
    }

    pub fn get_count(&self) -> i32 {
        self.entry.count.load(Ordering::Relaxed)
    }

    pub fn get_name(&self) -> &str {
        &self.entry.name
    }
}

/// Lifecycle-counting guard for a type.
#[derive(Debug)]
pub struct CountedObject<T> {
    counter: Counter,
    _marker: PhantomData<T>,
}

impl<T> CountedObject<T> {
    /// Create a counted-object guard using the Rust type's short name.
    ///
    /// This is convenient for internal tests and small wrappers. For migrated
    /// types whose names are surfaced through compatibility boundaries like
    /// `get_counts`, prefer `new_named` so the display name stays aligned with
    /// the reference implementation instead of depending on Rust module paths.
    pub fn new() -> Self {
        Self::with_counter(CountedObjects::get_instance().counter_for_type::<T>())
    }

    /// Create a counted-object guard with an explicit externally visible name.
    ///
    /// The first name chosen for a given `T` wins for the process lifetime,
    /// mirroring the one-static-counter-per-type behavior in the reference.
    pub fn new_named(name: impl Into<String>) -> Self {
        Self::with_counter(CountedObjects::get_instance().counter_for_type_named::<T>(name))
    }

    pub fn counter(&self) -> &Counter {
        &self.counter
    }

    fn with_counter(counter: Counter) -> Self {
        counter.increment();

        Self {
            counter,
            _marker: PhantomData,
        }
    }
}

impl<T> Default for CountedObject<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for CountedObject<T> {
    fn clone(&self) -> Self {
        Self::with_counter(self.counter.clone())
    }
}

impl<T> Drop for CountedObject<T> {
    fn drop(&mut self) {
        self.counter.decrement();
    }
}

/// Convenience wrapper that owns a value and counts it automatically.
#[derive(Debug)]
pub struct Counted<T> {
    inner: T,
    counted: CountedObject<T>,
}

impl<T> Counted<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            counted: CountedObject::new(),
        }
    }

    /// Create an owned counted value with an explicit external counter name.
    pub fn new_named(inner: T, name: impl Into<String>) -> Self {
        Self {
            inner,
            counted: CountedObject::new_named(name),
        }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn counter(&self) -> &Counter {
        self.counted.counter()
    }
}

impl<T> Clone for Counted<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            counted: self.counted.clone(),
        }
    }
}

impl<T> Deref for Counted<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for Counted<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

fn short_type_name<T>() -> String {
    type_name::<T>()
        .rsplit("::")
        .next()
        .expect("type name should not be empty")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Counted, CountedObject, CountedObjects, Counter};

    #[derive(Debug, Clone)]
    struct Transaction {
        _counted: CountedObject<Transaction>,
    }

    impl Transaction {
        fn new() -> Self {
            Self {
                _counted: CountedObject::new_named("Transaction"),
            }
        }
    }

    fn count_of(name: &str) -> i32 {
        CountedObjects::get_instance()
            .get_counts(0)
            .into_iter()
            .find_map(|(entry_name, count)| (entry_name == name).then_some(count))
            .unwrap_or(0)
    }

    #[test]
    fn counted_object_tracks_live_instances_and_clones() {
        let before = count_of("Transaction");

        let first = Transaction::new();
        assert_eq!(count_of("Transaction"), before + 1);

        let second = first.clone();
        assert_eq!(count_of("Transaction"), before + 2);

        drop(second);
        assert_eq!(count_of("Transaction"), before + 1);

        drop(first);
        assert_eq!(count_of("Transaction"), before);
    }

    #[test]
    fn counted_wrapper_counts_owned_values() {
        let before = count_of("Answer");

        let wrapped = Counted::new_named(42u32, "Answer");
        assert_eq!(count_of("Answer"), before + 1);
        assert_eq!(*wrapped, 42);

        let cloned = wrapped.clone();
        assert_eq!(count_of("Answer"), before + 2);
        assert_eq!(*cloned, 42);

        drop(cloned);
        drop(wrapped);
        assert_eq!(count_of("Answer"), before);
    }

    #[test]
    fn named_counters_are_reported_and_sorted_with_threshold() {
        let alpha = Counter::new("AlphaCounter");
        let beta = Counter::new("BetaCounter");

        let alpha_before = alpha.get_count();
        let beta_before = beta.get_count();

        alpha.increment();
        alpha.increment();
        beta.increment();

        let counts = CountedObjects::get_instance().get_counts(alpha_before + 2);
        assert!(
            counts
                .iter()
                .any(|(name, count)| name == "AlphaCounter" && *count == alpha_before + 2)
        );
        assert!(
            !counts
                .iter()
                .any(|(name, _)| name == "BetaCounter" && beta_before + 1 < alpha_before + 2)
        );

        let all_counts = CountedObjects::get_instance().get_counts(0);
        let mut sorted = all_counts.clone();
        sorted.sort();
        assert_eq!(all_counts, sorted);

        alpha.decrement();
        alpha.decrement();
        beta.decrement();
    }

    #[test]
    fn distinct_named_counters_with_same_label_stay_distinct() {
        let first = Counter::new("SharedName");
        let second = Counter::new("SharedName");

        first.increment();
        second.increment();

        let shared_entries = CountedObjects::get_instance()
            .get_counts(1)
            .into_iter()
            .filter(|(name, count)| name == "SharedName" && *count == 1)
            .count();

        assert!(shared_entries >= 2);

        first.decrement();
        second.decrement();
    }
}
