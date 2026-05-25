//! Rust equivalents of selected helpers from `xrpl/basics/algorithm.h`.

/// For each equivalent pair in two sorted ranges, call `action`.
///
/// This implementation keeps both ranges at the same element type. That maps
/// cleanly to Rust closure typing and covers the current compatibility tests.
pub fn generalized_set_intersection<T, Action, Comp>(
    left: &[T],
    right: &[T],
    mut action: Action,
    mut comp: Comp,
) where
    Action: FnMut(&T, &T),
    Comp: FnMut(&T, &T) -> bool,
{
    let mut i = 0;
    let mut j = 0;

    while i < left.len() && j < right.len() {
        if comp(&left[i], &right[j]) {
            i += 1;
        } else {
            if !comp(&right[j], &left[i]) {
                action(&left[i], &right[j]);
                i += 1;
            }
            j += 1;
        }
    }
}

/// Remove items from a sorted left range if they intersect a sorted right
/// range or match `pred`. Returns the new logical length.
pub fn remove_if_intersect_or_match<T, Pred, Comp>(
    left: &mut Vec<T>,
    right: &[T],
    mut pred: Pred,
    mut comp: Comp,
) -> usize
where
    Pred: FnMut(&T) -> bool,
    Comp: FnMut(&T, &T) -> bool,
{
    let original = std::mem::take(left);
    let mut preserved = Vec::with_capacity(original.len());
    let mut j = 0;

    for item in original {
        while j < right.len() && comp(&right[j], &item) {
            j += 1;
        }

        let intersects = j < right.len() && !comp(&item, &right[j]) && !comp(&right[j], &item);

        if !intersects && !pred(&item) {
            preserved.push(item);
        }
    }

    *left = preserved;
    left.len()
}

#[cfg(test)]
mod tests {
    use super::{generalized_set_intersection, remove_if_intersect_or_match};

    #[test]
    fn generalized_set_intersection_matches_header_algorithm() {
        let left = [1, 2, 3, 5, 8];
        let right = [0, 2, 3, 4, 8, 13];
        let mut seen = Vec::new();

        generalized_set_intersection(&left, &right, |a, b| seen.push((*a, *b)), |a, b| a < b);

        assert_eq!(seen, vec![(2, 2), (3, 3), (8, 8)]);
    }

    #[test]
    fn generalized_set_intersection_handles_duplicate_right_values() {
        let left = [2, 2, 3];
        let right = [2, 2, 2, 3];
        let mut seen = Vec::new();

        generalized_set_intersection(&left, &right, |a, b| seen.push((*a, *b)), |a, b| a < b);

        assert_eq!(seen, vec![(2, 2), (2, 2), (3, 3)]);
    }

    #[test]
    fn remove_if_intersect_or_match_removes_predicates_and_intersections() {
        let mut left = vec![1, 2, 3, 4, 5, 6];
        let right = [2, 4, 6, 8];

        let new_len =
            remove_if_intersect_or_match(&mut left, &right, |value| value % 5 == 0, |a, b| a < b);

        assert_eq!(new_len, 2);
        assert_eq!(left, vec![1, 3]);
    }

    #[test]
    fn remove_if_intersect_or_match_preserves_order_of_kept_items() {
        let mut left = vec![1, 3, 5, 7, 9];
        let right = [2, 4, 8];

        let new_len =
            remove_if_intersect_or_match(&mut left, &right, |value| *value == 7, |a, b| a < b);

        assert_eq!(new_len, 4);
        assert_eq!(left, vec![1, 3, 5, 9]);
    }
}
