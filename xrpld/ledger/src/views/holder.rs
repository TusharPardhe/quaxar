//! Thread-safe `LedgerHolder` compatibility surface.
//!
//! This mirrors the the reference implementation holder contract:
//! - hold one ledger behind a mutex,
//! - reject null input,
//! - reject mutable ledgers,
//! - return the currently held immutable ledger if present,
//! - report whether the holder is empty.

use crate::Ledger;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct LedgerHolder {
    held_ledger: Mutex<Option<Arc<Ledger>>>,
}

impl LedgerHolder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, ledger: Option<Arc<Ledger>>) {
        let ledger = ledger.expect("LedgerHolder::set with nullptr");
        assert!(
            ledger.is_immutable(),
            "LedgerHolder::set with mutable Ledger"
        );

        let mut held_ledger = self
            .held_ledger
            .lock()
            .expect("LedgerHolder mutex must not be poisoned");
        *held_ledger = Some(ledger);
    }

    pub fn get(&self) -> Option<Arc<Ledger>> {
        self.held_ledger
            .lock()
            .expect("LedgerHolder mutex must not be poisoned")
            .clone()
    }

    pub fn empty(&self) -> bool {
        self.held_ledger
            .lock()
            .expect("LedgerHolder mutex must not be poisoned")
            .is_none()
    }
}
