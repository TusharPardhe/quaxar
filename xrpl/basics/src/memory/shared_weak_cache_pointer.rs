//! Rust port of `xrpl/basics/SharedWeakCachePointer.h`.
//!
//! This mirrors the reference role closely:
//! - hold either a strong `Arc<T>` or a weak `Weak<T>`,
//! - expose explicit strong/weak state queries,
//! - support conversion in both directions,
//! - avoid storing both pointers at once.

use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct SharedWeakCachePointer<T> {
    combo: SharedWeak<T>,
}

#[derive(Debug)]
enum SharedWeak<T> {
    Strong(Arc<T>),
    Weak(Weak<T>),
    EmptyStrong,
}

impl<T> Default for SharedWeakCachePointer<T> {
    fn default() -> Self {
        Self {
            combo: SharedWeak::EmptyStrong,
        }
    }
}

impl<T> Clone for SharedWeakCachePointer<T> {
    fn clone(&self) -> Self {
        Self {
            combo: match &self.combo {
                SharedWeak::Strong(value) => SharedWeak::Strong(Arc::clone(value)),
                SharedWeak::Weak(value) => SharedWeak::Weak(value.clone()),
                SharedWeak::EmptyStrong => SharedWeak::EmptyStrong,
            },
        }
    }
}

impl<T> From<Arc<T>> for SharedWeakCachePointer<T> {
    fn from(value: Arc<T>) -> Self {
        Self {
            combo: SharedWeak::Strong(value),
        }
    }
}

impl<T> SharedWeakCachePointer<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_arc(value: Arc<T>) -> Self {
        Self::from(value)
    }

    pub fn get_strong(&self) -> Option<&Arc<T>> {
        match &self.combo {
            SharedWeak::Strong(value) => Some(value),
            SharedWeak::Weak(_) | SharedWeak::EmptyStrong => None,
        }
    }

    pub fn strong_clone(&self) -> Option<Arc<T>> {
        self.get_strong().map(Arc::clone)
    }

    pub fn is_strong(&self) -> bool {
        matches!(&self.combo, SharedWeak::Strong(_))
    }

    pub fn is_weak(&self) -> bool {
        !self.is_strong()
    }

    pub fn reset(&mut self) {
        self.combo = SharedWeak::EmptyStrong;
    }

    pub fn get(&self) -> Option<&T> {
        self.get_strong().map(Arc::as_ref)
    }

    pub fn use_count(&self) -> usize {
        self.get_strong().map_or(0, Arc::strong_count)
    }

    pub fn expired(&self) -> bool {
        match &self.combo {
            SharedWeak::Strong(_) => false,
            SharedWeak::Weak(value) => value.strong_count() == 0,
            SharedWeak::EmptyStrong => false,
        }
    }

    pub fn lock(&self) -> Option<Arc<T>> {
        match &self.combo {
            SharedWeak::Strong(value) => Some(Arc::clone(value)),
            SharedWeak::Weak(value) => value.upgrade(),
            SharedWeak::EmptyStrong => None,
        }
    }

    pub fn convert_to_strong(&mut self) -> bool {
        if self.is_strong() {
            return true;
        }

        match &self.combo {
            SharedWeak::Weak(value) => {
                if let Some(strong) = value.upgrade() {
                    self.combo = SharedWeak::Strong(strong);
                    return true;
                }
            }
            SharedWeak::Strong(_) => return true,
            SharedWeak::EmptyStrong => {}
        }

        false
    }

    pub fn convert_to_weak(&mut self) -> bool {
        match &self.combo {
            SharedWeak::Strong(value) => {
                self.combo = SharedWeak::Weak(Arc::downgrade(value));
                true
            }
            SharedWeak::Weak(_) | SharedWeak::EmptyStrong => true,
        }
    }

    pub fn set_strong(&mut self, value: Arc<T>) {
        self.combo = SharedWeak::Strong(value);
    }
}

impl<T> From<&SharedWeakCachePointer<T>> for bool {
    fn from(value: &SharedWeakCachePointer<T>) -> Self {
        value.is_strong()
    }
}

#[cfg(test)]
mod tests {
    use super::SharedWeakCachePointer;
    use std::sync::Arc;

    #[test]
    fn strong_and_weak_transitions_match_cpp_role() {
        let value = Arc::new(String::from("node"));
        let mut pointer = SharedWeakCachePointer::from_arc(Arc::clone(&value));

        assert!(pointer.is_strong());
        assert!(!pointer.is_weak());
        assert_eq!(
            pointer.get().map(|value| value.to_owned()),
            Some(String::from("node"))
        );
        assert!(pointer.use_count() >= 2);

        assert!(pointer.convert_to_weak());
        assert!(pointer.is_weak());
        assert_eq!(pointer.use_count(), 0);
        assert!(!pointer.expired());

        assert!(pointer.convert_to_strong());
        assert!(pointer.is_strong());

        drop(value);
        assert!(pointer.convert_to_weak());
        assert!(pointer.expired());
        assert!(!pointer.convert_to_strong());
    }

    #[test]
    fn reset_clears_pointer() {
        let mut pointer = SharedWeakCachePointer::from_arc(Arc::new(7u32));
        pointer.reset();

        assert!(pointer.is_weak());
        assert!(!pointer.expired());
        assert_eq!(pointer.lock(), None);
        assert_eq!(pointer.use_count(), 0);
        assert!(!pointer.convert_to_strong());
    }

    #[test]
    fn default_matches_empty_shared_variant_role() {
        let pointer = SharedWeakCachePointer::<u32>::new();

        assert!(pointer.is_weak());
        assert!(!pointer.is_strong());
        assert!(!pointer.expired());
        assert_eq!(pointer.use_count(), 0);
        assert_eq!(pointer.lock(), None);
    }
}
