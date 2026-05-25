//! Rust equivalent of `xrpl/basics/join.h`.
//!
//! The reference helper writes joined values into a stream. In Rust, the direct
//! equivalent is to build a `String` or provide a `Display` wrapper.

use std::fmt::{self, Display, Write};

/// Join displayable items with a delimiter.
pub fn join<I, T>(items: I, delimiter: &str) -> String
where
    I: IntoIterator<Item = T>,
    T: Display,
{
    let mut iter = items.into_iter();
    let mut output = String::new();

    if let Some(first) = iter.next() {
        write!(&mut output, "{first}").expect("writing to String cannot fail");
        for item in iter {
            output.push_str(delimiter);
            write!(&mut output, "{item}").expect("writing to String cannot fail");
        }
    }

    output
}

/// Display wrapper similar in role to reference `CollectionAndDelimiter`.
pub struct Joined<'a, T> {
    collection: &'a [T],
    delimiter: &'a str,
}

impl<'a, T> Joined<'a, T> {
    pub fn new(collection: &'a [T], delimiter: &'a str) -> Self {
        Self {
            collection,
            delimiter,
        }
    }
}

impl<T> Display for Joined<'_, T>
where
    T: Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&join(self.collection.iter(), self.delimiter))
    }
}

#[cfg(test)]
mod tests {
    use super::{Joined, join};

    #[test]
    fn join_matches_expected_delimiter_behavior() {
        assert_eq!(join(std::iter::empty::<i32>(), ", "), "");
        assert_eq!(join([1], ", "), "1");
        assert_eq!(join([1, 2, 3], ", "), "1, 2, 3");
    }

    #[test]
    fn join_supports_string_like_items() {
        assert_eq!(join(["a", "b", "c"], "-"), "a-b-c");
    }

    #[test]
    fn joined_wrapper_formats_stream_helper() {
        let values = [10, 20, 30];
        assert_eq!(format!("{}", Joined::new(&values, " | ")), "10 | 20 | 30");
    }
}
