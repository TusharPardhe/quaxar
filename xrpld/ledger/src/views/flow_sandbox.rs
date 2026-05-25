//! FlowSandbox — reference flow() internal sandbox parity.
//!
//! A child view that captures all writes locally. Can be applied to the parent
//! view on success, or discarded on failure. This matches reference flow() behavior
//! where the flow sandbox is only applied if the flow succeeds (finishFlow).

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{ApplyFlags, Keylet, Rules, STLedgerEntry, XRPAmount};

use crate::raw_view::RawView;
use crate::read_view::{ReadView, ReadViewTx, ViewError};
use crate::{ApplyView, Fees, LedgerHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Insert,
    Modify,
    Erase,
}

pub struct Entry {
    pub action: Action,
    pub sle: Arc<STLedgerEntry>,
}

/// A child view that captures writes locally and can be applied or discarded.
/// Matches reference flow() internal sandbox: only applied on tesSUCCESS via finishFlow.
pub struct FlowSandbox<'a, V: ApplyView + ?Sized> {
    parent: &'a mut V,
    items: BTreeMap<Uint256, Entry>,
    drops_destroyed: XRPAmount,
}

impl<'a, V: ApplyView + ?Sized> FlowSandbox<'a, V> {
    pub fn new(parent: &'a mut V) -> Self {
        Self {
            parent,
            items: BTreeMap::new(),
            drops_destroyed: XRPAmount::from_drops(0),
        }
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &BTreeMap<Uint256, Entry> {
        &self.items
    }

    pub fn peek_parent(&self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        self.parent.read(k)
    }

    /// Apply all captured changes to the parent view. Call on tesSUCCESS.
    pub fn apply(self) -> Result<(), ViewError> {
        for (_key, entry) in self.items {
            match entry.action {
                Action::Insert => {
                    self.parent.insert(entry.sle)?;
                }
                Action::Modify => {
                    self.parent.update(entry.sle)?;
                }
                Action::Erase => {
                    self.parent.erase(entry.sle)?;
                }
            }
        }
        if self.drops_destroyed.drops() > 0 {
            self.parent.destroy_xrp(self.drops_destroyed)?;
        }
        Ok(())
    }
}

impl<'a, V: ApplyView + ?Sized> std::fmt::Debug for FlowSandbox<'a, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowSandbox")
            .field("items", &self.items.len())
            .finish()
    }
}

impl<'a, V: ApplyView + ?Sized> ReadView for FlowSandbox<'a, V> {
    fn open(&self) -> bool {
        self.parent.open()
    }
    fn header(&self) -> LedgerHeader {
        self.parent.header()
    }
    fn fees(&self) -> Fees {
        self.parent.fees()
    }
    fn rules(&self) -> Rules {
        self.parent.rules()
    }

    fn exists(&self, k: Keylet) -> Result<bool, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            return Ok(entry.action != Action::Erase);
        }
        self.parent.exists(k)
    }

    fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
        let mut next = Some(key);
        loop {
            next = self.parent.succ(next.unwrap(), last)?;
            let Some(n) = next else { break };
            if let Some(entry) = self.items.get(&n)
                && entry.action == Action::Erase
            {
                continue;
            }
            break;
        }
        for (item_key, entry) in self
            .items
            .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
        {
            if entry.action != Action::Erase {
                if next.is_none() || next.unwrap() > *item_key {
                    next = Some(*item_key);
                }
                break;
            }
        }
        if let Some(n) = next
            && let Some(l) = last
            && n >= l
        {
            return Ok(None);
        }
        Ok(next)
    }

    fn read(&self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            if entry.action == Action::Erase {
                return Ok(None);
            }
            return Ok(Some(entry.sle.clone()));
        }
        self.parent.read(k)
    }

    fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
        self.parent.sles()
    }
    fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
        self.parent.tx_exists(key)
    }
    fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
        self.parent.tx_read(key)
    }
    fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
        self.parent.txs()
    }
}

impl<'a, V: ApplyView + ?Sized> RawView for FlowSandbox<'a, V> {
    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.items.insert(
            *sle.key(),
            Entry {
                action: Action::Insert,
                sle,
            },
        );
        Ok(())
    }
    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        let key = *sle.key();
        if let Some(existing) = self.items.get(&key)
            && existing.action == Action::Insert
        {
            self.items.insert(
                key,
                Entry {
                    action: Action::Insert,
                    sle,
                },
            );
            return Ok(());
        }
        self.items.insert(
            key,
            Entry {
                action: Action::Modify,
                sle,
            },
        );
        Ok(())
    }
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        let key = *sle.key();
        if let Some(existing) = self.items.get(&key)
            && existing.action == Action::Insert
        {
            self.items.remove(&key);
            return Ok(());
        }
        self.items.insert(
            key,
            Entry {
                action: Action::Erase,
                sle,
            },
        );
        Ok(())
    }
    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        self.drops_destroyed = XRPAmount::from_drops(self.drops_destroyed.drops() + fee.drops());
        Ok(())
    }
}

impl<'a, V: ApplyView + ?Sized> ApplyView for FlowSandbox<'a, V> {
    fn flags(&self) -> ApplyFlags {
        self.parent.flags()
    }
    fn peek(&mut self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            if entry.action == Action::Erase {
                return Ok(None);
            }
            return Ok(Some(entry.sle.clone()));
        }
        self.parent.peek(k)
    }
    fn insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.raw_insert(sle)
    }
    fn update(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.raw_replace(sle)
    }
    fn erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.raw_erase(sle)
    }
    fn destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        self.raw_destroy_xrp(fee)
    }
}
