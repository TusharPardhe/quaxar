//! Rust port of `xrpl::detail::RawStateTable` from `xrpl/ledger/detail/RawStateTable.h`.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{Keylet, STLedgerEntry, XRPAmount};

use crate::raw_view::RawView;
use crate::read_view::{ReadView, ViewError};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Erase,
    Insert,
    Replace,
}

#[derive(Debug, Clone)]
struct SleAction {
    action: Action,
    sle: Arc<STLedgerEntry>,
}

#[derive(Debug, Clone)]
pub struct RawStateTable {
    items: BTreeMap<Uint256, SleAction>,
    drops_destroyed: XRPAmount,
}

impl Default for RawStateTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RawStateTable {
    fn invariant_error(message: &str) -> ViewError {
        ViewError::Conversion(message.to_string())
    }

    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
            drops_destroyed: XRPAmount::new(),
        }
    }

    pub fn exists(&self, base: &dyn ReadView, k: Keylet) -> Result<bool, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            if entry.action == Action::Erase {
                return Ok(false);
            }
            return Ok(k.check_ledger_entry(entry.sle.get_type(), *entry.sle.key()));
        }
        base.exists(k)
    }

    pub fn succ(
        &self,
        base: &dyn ReadView,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, ViewError> {
        let mut next = Some(key);

        loop {
            next = match next {
                Some(current) => base.succ(current, last)?,
                None => None,
            };
            let Some(candidate) = next else {
                break;
            };
            if !matches!(self.items.get(&candidate), Some(entry) if entry.action == Action::Erase) {
                break;
            }
        }

        for (overlay_key, entry) in self
            .items
            .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
        {
            if entry.action != Action::Erase {
                if next.is_none_or(|candidate| candidate > *overlay_key) {
                    next = Some(*overlay_key);
                }
                break;
            }
        }

        if last.is_some_and(|last| next.is_some_and(|candidate| candidate >= last)) {
            return Ok(None);
        }

        Ok(next)
    }

    pub fn read(
        &self,
        base: &dyn ReadView,
        k: Keylet,
    ) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            if entry.action == Action::Erase {
                return Ok(None);
            }
            return Ok(k
                .check_ledger_entry(entry.sle.get_type(), *entry.sle.key())
                .then(|| entry.sle.clone()));
        }
        base.read(k)
    }

    pub fn erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        match self.items.get(sle.key()).map(|entry| &entry.action) {
            None => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Erase,
                        sle,
                    },
                );
            }
            Some(Action::Erase) => {
                return Err(Self::invariant_error(
                    "RawStateTable::erase: already erased",
                ));
            }
            Some(Action::Insert) => {
                self.items.remove(sle.key());
            }
            Some(Action::Replace) => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Erase,
                        sle,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        match self.items.get(sle.key()).map(|entry| &entry.action) {
            None => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Insert,
                        sle,
                    },
                );
            }
            Some(Action::Erase) => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Replace,
                        sle,
                    },
                );
            }
            Some(Action::Insert) => {
                return Err(Self::invariant_error(
                    "RawStateTable::insert: already inserted",
                ));
            }
            Some(Action::Replace) => {
                return Err(Self::invariant_error(
                    "RawStateTable::insert: already exists",
                ));
            }
        }
        Ok(())
    }

    pub fn replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        match self.items.get(sle.key()).map(|entry| &entry.action) {
            Some(Action::Erase) => {
                return Err(Self::invariant_error("RawStateTable::replace: was erased"));
            }
            Some(Action::Insert) => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Insert,
                        sle,
                    },
                );
            }
            None | Some(Action::Replace) => {
                self.items.insert(
                    *sle.key(),
                    SleAction {
                        action: Action::Replace,
                        sle,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn destroy_xrp(&mut self, fee: XRPAmount) {
        self.drops_destroyed += fee;
    }

    pub fn apply(&self, to: &mut dyn RawView) -> Result<(), ViewError> {
        to.raw_destroy_xrp(self.drops_destroyed)?;
        for entry in self.items.values() {
            match entry.action {
                Action::Erase => to.raw_erase(entry.sle.clone())?,
                Action::Insert => to.raw_insert(entry.sle.clone())?,
                Action::Replace => to.raw_replace(entry.sle.clone())?,
            }
        }
        Ok(())
    }
}
