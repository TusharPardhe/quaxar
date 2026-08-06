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
    flags: Option<ApplyFlags>,
}

impl<'a, V: ApplyView + ?Sized> FlowSandbox<'a, V> {
    pub fn new(parent: &'a mut V) -> Self {
        Self {
            parent,
            items: BTreeMap::new(),
            drops_destroyed: XRPAmount::from_drops(0),
            flags: None,
        }
    }

    /// Create a child view with explicit apply flags for this transaction
    /// attempt. Used by consensus retry passes to provide rippled's TapRetry
    /// semantics while retaining the parent accumulator's normal flags.
    pub fn new_with_flags(parent: &'a mut V, flags: ApplyFlags) -> Self {
        Self {
            parent,
            items: BTreeMap::new(),
            drops_destroyed: XRPAmount::from_drops(0),
            flags: Some(flags),
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

    /// Apply all captured changes after threading each inserted or modified
    /// SLE with the current transaction. This is the per-transaction
    /// `ApplyStateTable::threadItem` equivalent used by ledger acceptance;
    /// the parent remains the cumulative view for subsequent transactions.
    pub fn apply_with_tx_thread(
        mut self,
        tx_id: Uint256,
        ledger_seq: u32,
        rules: &Rules,
    ) -> Result<(), ViewError> {
        self.validate_parent_commit("FlowSandbox::apply_with_tx_thread")?;
        // `TapDryRun` must not burn the immutable ledger's XRP total.
        if self.drops_destroyed.drops() > 0
            && !protocol::any_apply_flags(self.flags() & ApplyFlags::DRY_RUN)
        {
            self.parent.destroy_xrp(self.drops_destroyed)?;
        }
        for (key, entry) in self.items {
            match entry.action {
                Action::Insert | Action::Modify => {
                    let threaded = Arc::new(crate::apply_state_table::thread_sle(
                        entry.sle.as_ref(),
                        tx_id,
                        ledger_seq,
                        rules,
                    ));
                    if entry.action == Action::Insert {
                        self.parent.insert(threaded)?;
                    } else {
                        let keylet = Keylet::new(threaded.get_type(), key);
                        if self.parent.peek(keylet)?.is_none() {
                            return Err(ViewError::Conversion(
                                "FlowSandbox::apply_with_tx_thread: modified parent entry disappeared"
                                    .into(),
                            ));
                        }
                        self.parent.update(threaded)?;
                    }
                }
                Action::Erase => {
                    let keylet = Keylet::new(entry.sle.get_type(), key);
                    if self.parent.peek(keylet)?.is_none() {
                        return Err(ViewError::Conversion(
                            "FlowSandbox::apply_with_tx_thread: erased parent entry disappeared"
                                .into(),
                        ));
                    }
                    self.parent.erase(entry.sle)?;
                }
            }
        }
        Ok(())
    }

    /// Apply all captured changes to the parent view. Call on tesSUCCESS.
    pub fn apply(mut self) -> Result<(), ViewError> {
        self.validate_parent_commit("FlowSandbox::apply")?;
        // `TapDryRun` must not burn the immutable ledger's XRP total.
        if self.drops_destroyed.drops() > 0
            && !protocol::any_apply_flags(self.flags() & ApplyFlags::DRY_RUN)
        {
            self.parent.destroy_xrp(self.drops_destroyed)?;
        }
        for (key, entry) in self.items {
            match entry.action {
                Action::Insert => {
                    self.parent.insert(entry.sle)?;
                }
                Action::Modify => {
                    // A FlowSandbox may have obtained this SLE through `read`,
                    // which deliberately does not populate the parent's
                    // ApplyStateTable. `ApplyView::update` requires that
                    // checkout, so make it explicit before forwarding the
                    // child modification. This is essential for directory
                    // pages changed by one transaction to remain visible to
                    // later transactions in the same ledger build.
                    let keylet = Keylet::new(entry.sle.get_type(), key);
                    if self.parent.peek(keylet)?.is_none() {
                        return Err(ViewError::Conversion(
                            "FlowSandbox::apply: modified parent entry disappeared".into(),
                        ));
                    }
                    self.parent.update(entry.sle)?;
                }
                Action::Erase => {
                    // As with Modify, erase must first establish the parent's
                    // checkout. Otherwise an owner-directory removal can be
                    // rejected despite the child having resolved the page.
                    let keylet = Keylet::new(entry.sle.get_type(), key);
                    if self.parent.peek(keylet)?.is_none() {
                        return Err(ViewError::Conversion(
                            "FlowSandbox::apply: erased parent entry disappeared".into(),
                        ));
                    }
                    self.parent.erase(entry.sle)?;
                }
            }
        }
        Ok(())
    }

    /// Validate all parent-side preconditions before the first child mutation.
    /// This gives FlowSandbox the same all-or-nothing transaction boundary as
    /// ../rippled/src/libxrpl/ledger/ApplyViewImpl.cpp::ApplyViewImpl::apply:
    /// a missing later entry is reported while the parent remains unchanged.
    fn validate_parent_commit(&mut self, operation: &str) -> Result<(), ViewError> {
        for (key, entry) in &self.items {
            let keylet = Keylet::new(entry.sle.get_type(), *key);
            let existing = self.parent.read(keylet)?;
            match entry.action {
                Action::Insert if existing.is_some() => {
                    return Err(ViewError::Conversion(format!(
                        "{operation}: inserted parent entry already exists"
                    )));
                }
                Action::Modify | Action::Erase if existing.is_none() => {
                    return Err(ViewError::Conversion(format!(
                        "{operation}: parent entry disappeared"
                    )));
                }
                Action::Modify | Action::Erase => {
                    // Establish ApplyStateTable checkout without exposing a
                    // mutation, so the later update/erase cannot fail solely
                    // because the child read through a non-checking path.
                    let _ = self.parent.peek(keylet)?;
                }
                Action::Insert => {}
            }
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
        let key = *sle.key();
        // An erase followed by insert of the same key is a replacement of an
        // SLE that exists in the parent view (directory root/page recreation
        // after removing its final index is the canonical example). Propagate
        // it as Modify; treating it as Insert makes the parent ApplyStateTable
        // see an already-existing key and loses the replacement, leaving an
        // Offer SLE pointing at stale book-directory membership.
        let action = if self
            .items
            .get(&key)
            .is_some_and(|existing| existing.action == Action::Erase)
        {
            Action::Modify
        } else {
            Action::Insert
        };
        self.items.insert(key, Entry { action, sle });
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
        self.flags.unwrap_or_else(|| self.parent.flags())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ledger, Sandbox};
    use basics::base_uint::Uint256;
    use protocol::{ApplyFlags, LedgerEntryType};
    use std::sync::Arc;

    #[test]
    fn apply_checks_out_parent_directory_page_before_propagating_replace() {
        let page_keylet = Keylet::new(LedgerEntryType::DirectoryNode, Uint256::from_u64(88));
        let mut page = STLedgerEntry::new(page_keylet);
        page.set_field_u64(protocol::get_field_by_symbol("sfIndexNext"), 0);

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(page.clone()))
            .expect("seed parent directory page");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());

        {
            let mut child = FlowSandbox::new(&mut parent);
            // Read intentionally bypasses the parent ApplyStateTable checkout.
            let read_page = child
                .read(page_keylet)
                .expect("child read")
                .expect("directory page exists");
            let mut replacement = read_page.clone_as_object();
            replacement.set_field_u64(protocol::get_field_by_symbol("sfIndexNext"), 1);
            child
                .raw_replace(Arc::new(STLedgerEntry::from_stobject(
                    replacement,
                    page_keylet.key,
                )))
                .expect("stage page replacement");
            child.apply().expect("propagate directory page replacement");
        }

        assert_eq!(
            parent
                .read(page_keylet)
                .expect("read propagated page")
                .expect("directory page remains")
                .get_field_u64(protocol::get_field_by_symbol("sfIndexNext")),
            1
        );
    }

    #[test]
    fn failed_commit_leaves_parent_without_earlier_child_insert() {
        // ../rippled/src/libxrpl/tx/apply.cpp::applyTransaction exposes a
        // child view only after it passes all parent preconditions.
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::default());
        let inserted_key = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(1));
        let missing_key = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(2));

        let result = {
            let mut child = FlowSandbox::new(&mut parent);
            child
                .insert(Arc::new(STLedgerEntry::new(inserted_key)))
                .expect("stage insert");
            child
                .raw_replace(Arc::new(STLedgerEntry::new(missing_key)))
                .expect("stage invalid replacement");
            child.apply()
        };

        assert!(result.is_err());
        assert!(
            !parent
                .exists(inserted_key)
                .expect("read parent after failed commit"),
            "a failed FlowSandbox commit must not expose an earlier insert"
        );
    }

    #[test]
    fn dropped_sandbox_never_mutates_parent() {
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::default());
        let keylet = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(77));
        let entry = Arc::new(STLedgerEntry::new(keylet));

        {
            let mut dry = FlowSandbox::new(&mut parent);
            dry.insert(entry).expect("stage dry mutation");
            assert!(dry.exists(keylet).expect("read dry sandbox"));
        }

        assert!(!parent.exists(keylet).expect("parent stays unchanged"));
    }
}
