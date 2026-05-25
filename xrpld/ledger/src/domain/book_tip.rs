//! BookTip — funded offer stream over a book's directory pages.
//!
//! Port of reference `BookTip` from `xrpl/tx/paths/BookTip.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{Book, Keylet, STLedgerEntry, get_field_by_symbol};

use crate::read_view::ViewError;
use crate::views::apply_view::ApplyView;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

/// Quality extracted from a directory key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quality(pub u64);

impl Quality {
    /// Extract quality from a book directory key (last 8 bytes).
    pub fn from_directory_key(key: &Uint256) -> Self {
        let bytes = key.data();
        let q = u64::from_be_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
        ]);
        Quality(q)
    }
}

/// BookTip — iterates through offers in a book in quality order.
///
/// Each call to `step()` advances to the next offer. The current offer
/// is available via `entry()`.
pub struct BookTip {
    book_base: Uint256,
    book_end: Uint256,
    current_dir: Option<Uint256>,
    current_index: Option<Uint256>,
    current_entry: Option<Arc<STLedgerEntry>>,
    quality: Option<Quality>,
    valid: bool,
}

impl BookTip {
    /// Create a new BookTip for the given book.
    pub fn new(book: &Book) -> Self {
        let book_base = protocol::get_book_base(*book);
        let book_end = protocol::next_keylet(protocol::book_keylet(*book)).key;
        Self {
            book_base,
            book_end,
            current_dir: None,
            current_index: None,
            current_entry: None,
            quality: None,
            valid: false,
        }
    }

    /// Advance to the next offer in the book.
    ///
    /// Returns `true` if an offer was found, `false` if the book is exhausted.
    pub fn step<V: ApplyView>(&mut self, view: &mut V) -> Result<bool, ViewError> {
        // If we had a previous entry, delete it (consumed offer)
        if self.valid
            && let Some(entry) = self.current_entry.take()
        {
            let _ = view.erase(entry);
        }

        loop {
            // Find next directory page at or after current position
            let first_page = view.succ(self.book_base, Some(self.book_end))?;
            let Some(page_key) = first_page else {
                return Ok(false);
            };

            // Read the directory page
            let dir_keylet = Keylet {
                entry_type: protocol::LedgerEntryType::DirectoryNode,
                key: page_key,
            };
            if let Some(dir) = view.peek(dir_keylet)? {
                let indexes = dir.get_field_v256(sf("sfIndexes"));
                let entries = indexes.value();
                if let Some(&first_index) = entries.first() {
                    self.current_dir = Some(*dir.key());
                    self.current_index = Some(first_index);
                    self.current_entry = view.peek(protocol::offer_keylet_from_key(first_index))?;
                    self.quality = Some(Quality::from_directory_key(&page_key));
                    self.valid = true;

                    // Advance book_base past this directory for next iteration
                    self.book_base = page_key;
                    // Decrement to position just before next quality
                    let bytes = self.book_base.data_mut();
                    // Simple decrement of last byte
                    for i in (0..32).rev() {
                        if bytes[i] > 0 {
                            bytes[i] -= 1;
                            break;
                        }
                        bytes[i] = 0xFF;
                    }

                    return Ok(true);
                }
            }

            // Empty directory — advance past it
            self.book_base = page_key;
        }
    }

    /// Current offer entry, if any.
    pub fn entry(&self) -> Option<&Arc<STLedgerEntry>> {
        self.current_entry.as_ref()
    }

    /// Current quality level.
    pub fn quality(&self) -> Option<Quality> {
        self.quality
    }
}
