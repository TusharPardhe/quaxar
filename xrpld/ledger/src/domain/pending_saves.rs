//! `PendingSaves` tracks ledgers that are currently being written to the database.
//! Ported from `xrpld/app/ledger/PendingSaves.h`.

use basics::base_uint::Uint256;
use std::collections::HashSet;
use std::sync::Mutex;

pub struct PendingSaves {
    pending: Mutex<HashSet<Uint256>>,
}

impl PendingSaves {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashSet::new()),
        }
    }

    pub fn start(&self, hash: Uint256) -> bool {
        self.pending
            .lock()
            .expect("pending saves lock")
            .insert(hash)
    }

    pub fn finish(&self, hash: Uint256) {
        self.pending
            .lock()
            .expect("pending saves lock")
            .remove(&hash);
    }

    pub fn is_pending(&self, hash: Uint256) -> bool {
        self.pending
            .lock()
            .expect("pending saves lock")
            .contains(&hash)
    }
}

impl Default for PendingSaves {
    fn default() -> Self {
        Self::new()
    }
}
