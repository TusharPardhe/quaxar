//! Rust equivalent of `xrpl/basics/comparators.h`.
//!
//! The reference header is primarily an MSVC compatibility shim around standard
//! comparators. Rust does not need that workaround, so this module stays thin.

/// Thin comparator wrapper matching the role of `xrpl::less`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Less;

impl Less {
    pub fn compare<T>(&self, left: &T, right: &T) -> bool
    where
        T: Ord,
    {
        left < right
    }
}

/// Thin comparator wrapper matching the role of `xrpl::equal_to`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EqualTo;

impl EqualTo {
    pub fn compare<T>(&self, left: &T, right: &T) -> bool
    where
        T: PartialEq,
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::{EqualTo, Less};

    #[test]
    fn less_matches_standard_ordering() {
        let cmp = Less;
        assert!(cmp.compare(&1, &2));
        assert!(!cmp.compare(&2, &1));
        assert!(!cmp.compare(&2, &2));
    }

    #[test]
    fn equal_to_matches_standard_equality() {
        let cmp = EqualTo;
        assert!(cmp.compare(&"xrpl", &"xrpl"));
        assert!(!cmp.compare(&"xrpl", &"rust"));
    }
}
