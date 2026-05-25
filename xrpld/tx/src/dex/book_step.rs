//! `BookStep` — funded offer stream over a book's directory pages.

use protocol::{Quality, STLedgerEntry};
use std::sync::Arc;

/// Trait representing a step in an order book, providing a stream of offers.
pub trait BookStep {
    /// Returns the next available offer in the book, or None if empty.
    fn next_offer<V: ledger::views::apply_view::ApplyView>(
        &mut self,
        view: &mut V,
    ) -> Result<Option<Arc<STLedgerEntry>>, ledger::views::read_view::ViewError>;

    /// Returns the quality of the current tip of the book.
    fn quality(&self) -> Option<Quality>;
}

pub struct BookStepImpl {
    tip: ledger::views::bridge::BookTip,
}

impl BookStepImpl {
    pub fn new(book: protocol::Book) -> Self {
        Self {
            tip: ledger::views::bridge::BookTip::new(&book),
        }
    }
}

impl BookStep for BookStepImpl {
    fn next_offer<V: ledger::views::apply_view::ApplyView>(
        &mut self,
        view: &mut V,
    ) -> Result<Option<Arc<STLedgerEntry>>, ledger::views::read_view::ViewError> {
        if self.tip.step(view)? {
            Ok(self.tip.entry().cloned())
        } else {
            Ok(None)
        }
    }

    fn quality(&self) -> Option<Quality> {
        self.tip.quality().map(|q| Quality::from_value(q.0))
    }
}
