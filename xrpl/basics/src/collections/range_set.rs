//! Rust port of `xrpl/basics/RangeSet.h`.
//!
//! The reference implementation uses Boost ICL. This Rust port keeps only the
//! compatibility surface we currently exercise:
//! - closed intervals,
//! - merged disjoint interval storage,
//! - styled string conversion,
//! - parsing from styled strings,
//! - `prevMissing`.

use std::fmt;
use std::str::FromStr;

pub trait DiscreteValue: Copy + Ord + fmt::Display + FromStr {
    fn zero() -> Self;
    fn checked_add_one(self) -> Option<Self>;
    fn checked_sub_one(self) -> Option<Self>;
    fn to_i128(self) -> i128;
}

macro_rules! impl_discrete_value_unsigned {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DiscreteValue for $ty {
                fn zero() -> Self {
                    0
                }

                fn checked_add_one(self) -> Option<Self> {
                    self.checked_add(1)
                }

                fn checked_sub_one(self) -> Option<Self> {
                    self.checked_sub(1)
                }

                fn to_i128(self) -> i128 {
                    self as i128
                }
            }
        )+
    };
}

macro_rules! impl_discrete_value_signed {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DiscreteValue for $ty {
                fn zero() -> Self {
                    0
                }

                fn checked_add_one(self) -> Option<Self> {
                    self.checked_add(1)
                }

                fn checked_sub_one(self) -> Option<Self> {
                    self.checked_sub(1)
                }

                fn to_i128(self) -> i128 {
                    self as i128
                }
            }
        )+
    };
}

impl_discrete_value_unsigned!(u8, u16, u32, u64, usize);
impl_discrete_value_signed!(i8, i16, i32, i64, isize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedInterval<T> {
    first: T,
    last: T,
}

impl<T> ClosedInterval<T> {
    pub fn new(first: T, last: T) -> Self {
        Self { first, last }
    }

    pub fn first(&self) -> T
    where
        T: Copy,
    {
        self.first
    }

    pub fn last(&self) -> T
    where
        T: Copy,
    {
        self.last
    }
}

impl<T> fmt::Display for ClosedInterval<T>
where
    T: fmt::Display + PartialEq,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.first == self.last {
            write!(formatter, "{}", self.first)
        } else {
            write!(formatter, "{}-{}", self.first, self.last)
        }
    }
}

pub fn range<T>(low: T, high: T) -> ClosedInterval<T> {
    ClosedInterval::new(low, high)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSet<T> {
    intervals: Vec<ClosedInterval<T>>,
}

impl<T> Default for RangeSet<T> {
    fn default() -> Self {
        Self {
            intervals: Vec::new(),
        }
    }
}

impl<T> RangeSet<T>
where
    T: DiscreteValue,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn empty(&self) -> bool {
        self.intervals.is_empty()
    }

    pub fn clear(&mut self) {
        self.intervals.clear();
    }

    pub fn insert(&mut self, value: T) {
        self.insert_interval(range(value, value));
    }

    pub fn insert_interval(&mut self, interval: ClosedInterval<T>) {
        if interval.first > interval.last {
            return;
        }

        let mut pending = interval;
        let mut merged = Vec::with_capacity(self.intervals.len() + 1);
        let mut inserted = false;

        for current in &self.intervals {
            if strictly_before(*current, pending) {
                merged.push(*current);
            } else if strictly_before(pending, *current) {
                if !inserted {
                    merged.push(pending);
                    inserted = true;
                }
                merged.push(*current);
            } else {
                pending = range(
                    std::cmp::min(pending.first, current.first),
                    std::cmp::max(pending.last, current.last),
                );
            }
        }

        if !inserted {
            merged.push(pending);
        }

        self.intervals = merged;
    }

    pub fn erase_interval(&mut self, removed: ClosedInterval<T>) {
        if removed.first > removed.last {
            return;
        }

        let mut kept = Vec::with_capacity(self.intervals.len());

        for current in &self.intervals {
            if current.last < removed.first || current.first > removed.last {
                kept.push(*current);
                continue;
            }

            if current.first < removed.first
                && let Some(left_last) = removed.first.checked_sub_one()
            {
                kept.push(range(current.first, left_last));
            }

            if current.last > removed.last
                && let Some(right_first) = removed.last.checked_add_one()
            {
                kept.push(range(right_first, current.last));
            }
        }

        self.intervals = kept;
    }

    pub fn contains(&self, value: T) -> bool {
        self.intervals
            .iter()
            .any(|interval| interval.first <= value && value <= interval.last)
    }

    pub fn first(&self) -> Option<T> {
        self.intervals.first().map(ClosedInterval::first)
    }

    pub fn last(&self) -> Option<T> {
        self.intervals.last().map(ClosedInterval::last)
    }

    pub fn length(&self) -> usize {
        self.intervals
            .iter()
            .map(|interval| (interval.last.to_i128() - interval.first.to_i128() + 1) as usize)
            .sum()
    }

    pub fn intervals(&self) -> &[ClosedInterval<T>] {
        &self.intervals
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ClosedInterval<T>> {
        self.intervals.iter()
    }
}

impl<T> fmt::Display for RangeSet<T>
where
    T: DiscreteValue,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.empty() {
            return formatter.write_str("empty");
        }

        for (index, interval) in self.intervals.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{interval}")?;
        }

        Ok(())
    }
}

pub fn to_string_interval<T>(interval: ClosedInterval<T>) -> String
where
    T: fmt::Display + PartialEq,
{
    interval.to_string()
}

pub fn to_string_range_set<T>(range_set: &RangeSet<T>) -> String
where
    T: DiscreteValue,
{
    range_set.to_string()
}

pub fn from_string<T>(range_set: &mut RangeSet<T>, input: &str) -> bool
where
    T: DiscreteValue,
{
    range_set.clear();
    let mut parsed = RangeSet::new();

    for token in input.split(',') {
        let intervals = token.split('-').collect::<Vec<_>>();
        match intervals.as_slice() {
            [single] => match single.parse::<T>() {
                Ok(value) => parsed.insert(value),
                Err(_) => return fail_parse(range_set),
            },
            [front, back] => {
                let Ok(front) = front.parse::<T>() else {
                    return fail_parse(range_set);
                };
                let Ok(back) = back.parse::<T>() else {
                    return fail_parse(range_set);
                };
                parsed.insert_interval(range(front, back));
            }
            _ => return fail_parse(range_set),
        }
    }

    *range_set = parsed;
    true
}

pub fn prev_missing<T>(range_set: &RangeSet<T>, target: T, min_value: T) -> Option<T>
where
    T: DiscreteValue,
{
    if range_set.empty() || target == min_value {
        return None;
    }

    let upper = target.checked_sub_one()?;
    let mut candidate = RangeSet::new();
    candidate.insert_interval(range(min_value, upper));

    for interval in range_set.intervals() {
        candidate.erase_interval(*interval);
    }

    candidate.last()
}

fn fail_parse<T>(range_set: &mut RangeSet<T>) -> bool
where
    T: DiscreteValue,
{
    range_set.clear();
    false
}

fn strictly_before<T>(left: ClosedInterval<T>, right: ClosedInterval<T>) -> bool
where
    T: DiscreteValue,
{
    if left.last >= right.first {
        return false;
    }

    match left.last.checked_add_one() {
        Some(next) => next < right.first,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RangeSet, from_string, prev_missing, range, to_string_interval, to_string_range_set,
    };

    #[test]
    fn prev_missing_reference_cases() {
        let mut set = RangeSet::<u32>::new();
        for i in 0..10 {
            set.insert_interval(range(10 * i, 10 * i + 5));
        }

        for i in 1..100 {
            let expected = if i <= 6 {
                None
            } else {
                Some(if (i % 10) > 6 {
                    i - 1
                } else {
                    (10 * (i / 10)) - 1
                })
            };

            assert_eq!(prev_missing(&set, i, 0), expected);
        }
    }

    #[test]
    fn string_conversion_reference_cases() {
        let mut set = RangeSet::<u32>::new();
        assert_eq!(to_string_range_set(&set), "empty");

        set.insert(1);
        assert_eq!(to_string_interval(range(1u32, 1u32)), "1");
        assert_eq!(to_string_range_set(&set), "1");

        set.insert_interval(range(4u32, 6u32));
        assert_eq!(to_string_range_set(&set), "1,4-6");

        set.insert(2);
        assert_eq!(to_string_range_set(&set), "1-2,4-6");

        set.erase_interval(range(4u32, 5u32));
        assert_eq!(to_string_range_set(&set), "1-2,6");
    }

    #[test]
    fn parsing_reference_cases() {
        let mut set = RangeSet::<u32>::new();

        assert!(!from_string(&mut set, ""));
        assert_eq!(set.length(), 0);

        assert!(!from_string(&mut set, "#"));
        assert_eq!(set.length(), 0);

        assert!(!from_string(&mut set, ","));
        assert_eq!(set.length(), 0);

        assert!(!from_string(&mut set, ",-"));
        assert_eq!(set.length(), 0);

        assert!(!from_string(&mut set, "1,,2"));
        assert_eq!(set.length(), 0);

        assert!(from_string(&mut set, "1"));
        assert_eq!(set.length(), 1);
        assert_eq!(set.first(), Some(1));

        assert!(from_string(&mut set, "1,1"));
        assert_eq!(set.length(), 1);
        assert_eq!(set.first(), Some(1));

        assert!(from_string(&mut set, "1-1"));
        assert_eq!(set.length(), 1);
        assert_eq!(set.first(), Some(1));

        assert!(from_string(&mut set, "1,4-6"));
        assert_eq!(set.length(), 4);
        assert_eq!(set.first(), Some(1));
        assert!(!set.contains(2));
        assert!(!set.contains(3));
        assert!(set.contains(4));
        assert!(set.contains(5));
        assert_eq!(set.last(), Some(6));

        assert!(from_string(&mut set, "1-2,4-6"));
        assert_eq!(set.length(), 5);
        assert_eq!(set.first(), Some(1));
        assert!(set.contains(2));
        assert!(set.contains(4));
        assert_eq!(set.last(), Some(6));

        assert!(from_string(&mut set, "1-2,6"));
        assert_eq!(set.length(), 3);
        assert_eq!(set.first(), Some(1));
        assert!(set.contains(2));
        assert_eq!(set.last(), Some(6));
    }
}
