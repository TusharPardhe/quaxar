//! Bridge module to expose internal view helpers to other crates.
pub use crate::domain::book_tip::{BookTip, Quality};
pub use crate::views::apply_view::adjust_owner_count;
pub use crate::views::directory::{dir_append, dir_insert, dir_remove};
