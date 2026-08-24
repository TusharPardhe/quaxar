//! FlowSandbox — reference flow() internal sandbox parity.
//!
//! A child view that captures all writes locally. Can be applied to the parent
//! view on success, or discarded on failure. This matches reference flow() behavior
//! where the flow sandbox is only applied if the flow succeeds (finishFlow).

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    ApplyFlags, Keylet, LedgerEntryType, Rules, SField, STLedgerEntry, STObject, SerializedTypeId,
    StBase, XRPAmount, account_keylet, get_field_by_symbol,
};

use crate::raw_view::RawView;
use crate::read_view::{ReadView, ReadViewTx, ViewError};
use crate::{ApplyView, Fees, LedgerHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Insert,
    Modify,
    Erase,
}

#[derive(Clone)]
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

    /// Returns the first entry that proves this transaction was already
    /// threaded into an earlier ledger. Callers must discard the uncommitted
    /// sandbox rather than invoke `STLedgerEntry::thread` again with a new
    /// ledger sequence. This is the state-level backstop for rippled's
    /// `ReadView::txExists` replay rejection when a historical tx-map branch
    /// is unavailable.
    pub fn replayed_threaded_entry(
        &self,
        tx_id: Uint256,
        ledger_seq: u32,
        rules: &Rules,
    ) -> Option<(Uint256, u32)> {
        self.items.iter().find_map(|(key, entry)| {
            if !matches!(entry.action, Action::Insert | Action::Modify)
                || !entry.sle.is_threaded_type(rules)
            {
                return None;
            }
            let prior_id = entry
                .sle
                .get_field_h256(get_field_by_symbol("sfPreviousTxnID"));
            let prior_seq = entry
                .sle
                .get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"));
            (prior_id == tx_id && prior_seq != ledger_seq).then_some((*key, prior_seq))
        })
    }

    /// Build the transaction metadata from this transaction's uncommitted
    /// delta. This is deliberately callable before `apply_with_tx_thread`
    /// consumes the sandbox: rippled's
    /// `ApplyStateTable::apply(OpenView&, STTx const&, ...)` constructs
    /// `TxMeta`, serializes the TransactionMd leaf, and only then applies the
    /// state table to the accumulator.
    pub fn to_tx_meta(
        &self,
        transaction_id: Uint256,
        ledger_seq: u32,
        delivered_amount: Option<protocol::STAmount>,
        rules: &Rules,
    ) -> Result<protocol::TxMeta, ViewError> {
        let mut meta = protocol::TxMeta::new(transaction_id, ledger_seq);
        meta.set_delivered_amount(delivered_amount);

        // rippled mutates entries in its ordered items_ map while metadata is
        // being built: threadOwners can change an AccountRoot that has not yet
        // reached its own map position. Work on a shadow map so that exact
        // ordering is represented without consuming the real transaction
        // delta before commit.
        let mut items = self.items.clone();
        let mut new_mod = BTreeMap::<Uint256, Arc<STLedgerEntry>>::new();
        let keys = items.keys().copied().collect::<Vec<_>>();

        for key in keys {
            let entry = items
                .get(&key)
                .cloned()
                .expect("metadata key came from the shadow state table");
            match entry.action {
                Action::Insert => {
                    // ApplyStateTable::apply calls setAffectedNode for the
                    // current map item before threadOwners may append a
                    // supplemental AccountRoot. Preserve that insertion
                    // order for the unsorted ledgerDelta representation too.
                    let _ = meta.get_affected_node_for_sle(
                        entry.sle.as_ref(),
                        protocol::get_field_by_symbol("sfCreatedNode"),
                    );
                    self.thread_owners_for_metadata(
                        entry.sle.as_ref(),
                        &mut items,
                        &mut new_mod,
                        &mut meta,
                        transaction_id,
                        ledger_seq,
                        rules,
                    )?;
                    let entry = items
                        .get(&key)
                        .cloned()
                        .expect("created entry remains in the shadow state table");
                    // rippled threads created entries before it selects their
                    // `sfNewFields`, so the leaf and committed state agree.
                    let current = crate::apply_state_table::thread_sle(
                        entry.sle.as_ref(),
                        transaction_id,
                        ledger_seq,
                        rules,
                    );
                    let node = meta.get_affected_node_for_sle(
                        &current,
                        protocol::get_field_by_symbol("sfCreatedNode"),
                    );
                    add_threading_previous_fields(node, entry.sle.as_ref(), transaction_id, rules);
                    let (fields, selected) = metadata_fields(&current, |field| {
                        !field.is_default()
                            && field
                                .fname()
                                .should_meta(SField::S_MD_CREATE | SField::S_MD_ALWAYS)
                    });
                    if selected {
                        node.set_field_object(protocol::get_field_by_symbol("sfNewFields"), fields);
                    }
                    items.get_mut(&key).expect("created entry remains").sle = Arc::new(current);
                }
                Action::Modify => {
                    let original = self
                        .parent
                        .read(Keylet::new(entry.sle.get_type(), key))?
                        .ok_or_else(|| {
                            ViewError::Conversion(
                                "FlowSandbox::to_tx_meta: modified parent entry disappeared"
                                    .to_owned(),
                            )
                        })?;
                    // Source parity: ApplyStateTable skips an unchanged
                    // modification before `threadItem` mutates it.
                    if entry.sle.as_ref() == original.as_ref() {
                        continue;
                    }
                    let current = crate::apply_state_table::thread_sle(
                        entry.sle.as_ref(),
                        transaction_id,
                        ledger_seq,
                        rules,
                    );
                    let node = meta.get_affected_node_for_sle(
                        &current,
                        protocol::get_field_by_symbol("sfModifiedNode"),
                    );
                    add_threading_previous_fields(node, entry.sle.as_ref(), transaction_id, rules);
                    let (previous, previous_selected) =
                        metadata_fields(original.as_ref(), |field| {
                            field.fname().should_meta(SField::S_MD_CHANGE_ORIG)
                                && !field_matches(&current, field)
                        });
                    if previous_selected {
                        node.set_field_object(
                            protocol::get_field_by_symbol("sfPreviousFields"),
                            previous,
                        );
                    }
                    let (final_fields, final_fields_selected) =
                        metadata_fields(&current, |field| {
                            field
                                .fname()
                                .should_meta(SField::S_MD_ALWAYS | SField::S_MD_CHANGE_NEW)
                        });
                    if final_fields_selected {
                        node.set_field_object(
                            protocol::get_field_by_symbol("sfFinalFields"),
                            final_fields,
                        );
                    }
                    items.get_mut(&key).expect("modified entry remains").sle = Arc::new(current);
                }
                Action::Erase => {
                    let original = self
                        .parent
                        .read(Keylet::new(entry.sle.get_type(), key))?
                        .ok_or_else(|| {
                            ViewError::Conversion(
                                "FlowSandbox::to_tx_meta: erased parent entry disappeared"
                                    .to_owned(),
                            )
                        })?;
                    // As with CreatedNode, rippled registers the DeletedNode
                    // before owner threading can append metadata entries.
                    let _ = meta.get_affected_node_for_sle(
                        entry.sle.as_ref(),
                        protocol::get_field_by_symbol("sfDeletedNode"),
                    );
                    self.thread_owners_for_metadata(
                        original.as_ref(),
                        &mut items,
                        &mut new_mod,
                        &mut meta,
                        transaction_id,
                        ledger_seq,
                        rules,
                    )?;
                    let node = meta.get_affected_node_for_sle(
                        entry.sle.as_ref(),
                        protocol::get_field_by_symbol("sfDeletedNode"),
                    );
                    let (previous, previous_selected) =
                        metadata_fields(original.as_ref(), |field| {
                            field.fname().should_meta(SField::S_MD_CHANGE_ORIG)
                                && !field_matches(entry.sle.as_ref(), field)
                        });
                    if previous_selected {
                        node.set_field_object(
                            protocol::get_field_by_symbol("sfPreviousFields"),
                            previous,
                        );
                    }
                    let (final_fields, final_fields_selected) =
                        metadata_fields(entry.sle.as_ref(), |field| {
                            field
                                .fname()
                                .should_meta(SField::S_MD_ALWAYS | SField::S_MD_DELETE_FINAL)
                        });
                    if final_fields_selected {
                        node.set_field_object(
                            protocol::get_field_by_symbol("sfFinalFields"),
                            final_fields,
                        );
                    }
                }
            }
        }

        Ok(meta)
    }

    fn thread_owners_for_metadata(
        &self,
        owner_source: &STLedgerEntry,
        items: &mut BTreeMap<Uint256, Entry>,
        new_mod: &mut BTreeMap<Uint256, Arc<STLedgerEntry>>,
        meta: &mut protocol::TxMeta,
        transaction_id: Uint256,
        ledger_seq: u32,
        rules: &Rules,
    ) -> Result<(), ViewError> {
        for owner in owner_accounts(owner_source) {
            let keylet = account_keylet(Uint160::from_void(owner.data()));
            let unthreaded = if let Some(sle) = new_mod.get(&keylet.key) {
                Some(Arc::clone(sle))
            } else if let Some(entry) = items.get(&keylet.key) {
                (entry.action != Action::Erase).then(|| Arc::clone(&entry.sle))
            } else {
                self.parent.read(keylet)?
            };
            let Some(unthreaded) = unthreaded else {
                continue;
            };
            let current = Arc::new(crate::apply_state_table::thread_sle(
                unthreaded.as_ref(),
                transaction_id,
                ledger_seq,
                rules,
            ));
            let previous_transaction =
                unthreaded.get_field_h256(protocol::get_field_by_symbol("sfPreviousTxnID"));
            if unthreaded.is_threaded_type(rules)
                && !previous_transaction.is_zero()
                && previous_transaction != transaction_id
            {
                let node = meta.get_affected_node_for_sle(
                    current.as_ref(),
                    protocol::get_field_by_symbol("sfModifiedNode"),
                );
                add_threading_previous_fields(node, unthreaded.as_ref(), transaction_id, rules);
            }

            if let Some(entry) = items.get_mut(&keylet.key) {
                entry.sle = current;
            } else {
                new_mod.insert(keylet.key, current);
            }
        }
        Ok(())
    }

    /// Collect rippled ApplyStateTable's supplemental `newMod` AccountRoots.
    /// Created and deleted owner-bearing SLEs thread their sfAccount and
    /// sfDestination accounts; RippleState threads both issuers. A material
    /// mutation of the same AccountRoot wins and is threaded by the ordinary
    /// delta path.
    fn collect_owner_threads(&self) -> Result<BTreeMap<Uint256, Arc<STLedgerEntry>>, ViewError> {
        let mut owner_threads = BTreeMap::new();
        for (key, entry) in &self.items {
            let owner_source = match entry.action {
                Action::Insert => Some(Arc::clone(&entry.sle)),
                Action::Erase => self.parent.read(Keylet::new(entry.sle.get_type(), *key))?,
                Action::Modify => None,
            };
            let Some(owner_source) = owner_source else {
                continue;
            };

            for owner in owner_accounts(owner_source.as_ref()) {
                let keylet = account_keylet(Uint160::from_void(owner.data()));
                if owner_threads.contains_key(&keylet.key) {
                    continue;
                }
                let owner_sle = match self.items.get(&keylet.key) {
                    // A material mutation is threaded by the ordinary delta
                    // path. An unchanged Modify is different: rippled's main
                    // metadata loop may already have skipped it, but
                    // getForMod returns and threadOwners mutates that same
                    // item later, regardless of map iteration order.
                    Some(Entry {
                        action: Action::Modify,
                        sle,
                    }) => match self.parent.read(keylet)? {
                        Some(original) if original.as_ref() == sle.as_ref() => {
                            Some(Arc::clone(sle))
                        }
                        _ => continue,
                    },
                    // Inserts are always handled by the ordinary path.
                    Some(Entry {
                        action: Action::Insert,
                        ..
                    }) => continue,
                    // Deleted destinations are intentionally not restored.
                    Some(Entry {
                        action: Action::Erase,
                        ..
                    }) => continue,
                    None => self.parent.read(keylet)?,
                };
                if let Some(owner_sle) = owner_sle {
                    owner_threads.insert(keylet.key, owner_sle);
                }
            }
        }
        Ok(owner_threads)
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
        let owner_threads = self.collect_owner_threads()?;
        self.validate_parent_commit("FlowSandbox::apply_with_tx_thread")?;
        // `TapDryRun` must not burn the immutable ledger's XRP total.
        if self.drops_destroyed.drops() > 0
            && !protocol::any_apply_flags(self.flags() & ApplyFlags::DRY_RUN)
        {
            self.parent.destroy_xrp(self.drops_destroyed)?;
        }
        for (key, entry) in self.items {
            match entry.action {
                Action::Insert => {
                    let threaded = Arc::new(crate::apply_state_table::thread_sle(
                        entry.sle.as_ref(),
                        tx_id,
                        ledger_seq,
                        rules,
                    ));
                    self.parent.insert(threaded)?;
                }
                Action::Modify => {
                    let keylet = Keylet::new(entry.sle.get_type(), key);
                    let original = self.parent.peek(keylet)?.ok_or_else(|| {
                        ViewError::Conversion(
                            "FlowSandbox::apply_with_tx_thread: modified parent entry disappeared"
                                .into(),
                        )
                    })?;
                    // rippled skips a no-op ModifiedNode before threadItem.
                    // Keep state commit aligned with to_tx_meta's same check.
                    if entry.sle.as_ref() == original.as_ref() {
                        continue;
                    }
                    self.parent
                        .update(Arc::new(crate::apply_state_table::thread_sle(
                            entry.sle.as_ref(),
                            tx_id,
                            ledger_seq,
                            rules,
                        )))?;
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
        // Rust commits direct changes before supplemental owner threads.
        // Same-key material mutations are excluded above, so supplemental
        // copies cannot overwrite them and the resulting state matches
        // rippled's newMod-before-state-table application order.
        for (key, owner) in owner_threads {
            let keylet = Keylet::new(LedgerEntryType::AccountRoot, key);
            if self.parent.peek(keylet)?.is_none() {
                return Err(ViewError::Conversion(
                    "FlowSandbox::apply_with_tx_thread: owner account disappeared".into(),
                ));
            }
            self.parent
                .update(Arc::new(crate::apply_state_table::thread_sle(
                    owner.as_ref(),
                    tx_id,
                    ledger_seq,
                    rules,
                )))?;
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

fn owner_accounts(sle: &STLedgerEntry) -> Vec<protocol::AccountID> {
    if sle.get_type() == LedgerEntryType::AccountRoot {
        return Vec::new();
    }
    if sle.get_type() == LedgerEntryType::RippleState {
        return vec![
            sle.get_field_amount(get_field_by_symbol("sfLowLimit"))
                .issue()
                .issuer(),
            sle.get_field_amount(get_field_by_symbol("sfHighLimit"))
                .issue()
                .issuer(),
        ];
    }

    ["sfAccount", "sfDestination"]
        .into_iter()
        .filter_map(|name| {
            let field = get_field_by_symbol(name);
            sle.is_field_present(field)
                .then(|| sle.get_account_id(field))
        })
        .collect()
}

/// Copy the source's `for (auto const& obj : *sle)` metadata selection into a
/// serializable inner object. The boolean records whether rippled's typed-slot
/// loop selected anything. It deliberately differs from whether the result has
/// a serializable field: selecting a `NotPresent` template slot produces a
/// canonical empty object (`E6E1`) rather than no object at all.
fn metadata_fields(
    sle: &STLedgerEntry,
    mut include: impl FnMut(&dyn StBase) -> bool,
) -> (STObject, bool) {
    let mut fields = sle.clone_as_object();
    let mut selected = false;
    let absent = fields
        .iter()
        .filter(|field| {
            let keep = include(*field);
            selected |= keep;
            !keep || field.stype() == SerializedTypeId::NotPresent
        })
        .map(|field| field.fname())
        .collect::<Vec<_>>();
    for field in absent {
        fields.make_field_absent(field);
    }
    // `sfLedgerEntryType` identifies the outer affected-node object and is
    // never present inside NewFields, FinalFields, or PreviousFields.
    fields.make_field_absent(protocol::get_field_by_symbol("sfLedgerEntryType"));
    (fields, selected)
}

/// Preserve the outer fields that rippled's `ApplyStateTable::threadItem`
/// writes on an affected node before generating its PreviousFields object.
/// This only describes the already-planned threaded state; it never mutates
/// the FlowSandbox or its parent.
fn add_threading_previous_fields(
    node: &mut STObject,
    unthreaded: &STLedgerEntry,
    transaction_id: Uint256,
    rules: &Rules,
) {
    if !unthreaded.is_threaded_type(rules) {
        return;
    }

    let previous_transaction =
        unthreaded.get_field_h256(protocol::get_field_by_symbol("sfPreviousTxnID"));
    if previous_transaction.is_zero() || previous_transaction == transaction_id {
        return;
    }

    node.set_field_h256(
        protocol::get_field_by_symbol("sfPreviousTxnID"),
        previous_transaction,
    );
    node.set_field_u32(
        protocol::get_field_by_symbol("sfPreviousTxnLgrSeq"),
        unthreaded.get_field_u32(protocol::get_field_by_symbol("sfPreviousTxnLgrSeq")),
    );
}

/// Equivalent to rippled `curNode->hasMatchingEntry(obj)`.
fn field_matches(current: &STLedgerEntry, original_field: &dyn StBase) -> bool {
    current
        .peek_at_pfield(original_field.fname())
        .is_some_and(|field| {
            field.stype() == original_field.stype() && field.is_equivalent(original_field)
        })
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
    use protocol::{ApplyFlags, LedgerEntryType, StBase};
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
    fn delta_metadata_retains_insert_modify_and_erase_before_commit() {
        // rippled ApplyStateTable::apply builds TxMeta from the isolated
        // transaction delta before it commits the accumulated state table.
        let modify_keylet = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(11));
        let erase_keylet = Keylet::new(LedgerEntryType::Offer, Uint256::from_u64(12));
        let insert_keylet = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(13));

        let mut base = Ledger::new(LedgerHeader::default(), false);
        let mut modified = STLedgerEntry::new(modify_keylet);
        let previous_transaction = Uint256::from_u64(0xBEEF);
        modified.set_field_u32(protocol::get_field_by_symbol("sfSequence"), 1);
        modified.set_field_h256(
            protocol::get_field_by_symbol("sfPreviousTxnID"),
            previous_transaction,
        );
        modified.set_field_u32(protocol::get_field_by_symbol("sfPreviousTxnLgrSeq"), 41);
        base.raw_insert(Arc::new(modified.clone()))
            .expect("seed modified entry");
        base.raw_insert(Arc::new(STLedgerEntry::new(erase_keylet)))
            .expect("seed erased entry");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            let mut replacement = modified.clone_as_object();
            replacement.set_field_u32(protocol::get_field_by_symbol("sfSequence"), 2);
            delta
                .raw_replace(Arc::new(STLedgerEntry::from_stobject(
                    replacement,
                    modify_keylet.key,
                )))
                .expect("stage modified entry");
            delta
                .raw_erase(Arc::new(STLedgerEntry::new(erase_keylet)))
                .expect("stage erased entry");
            delta
                .raw_insert(Arc::new(STLedgerEntry::new(insert_keylet)))
                .expect("stage inserted entry");

            delta
                .to_tx_meta(Uint256::from_u64(0xA11CE), 42, None, &rules)
                .expect("delta metadata should build before commit")
        };

        let nodes = metadata.get_nodes();
        assert_eq!(nodes.len(), 3, "every state-delta action needs metadata");
        let modified_node = nodes
            .iter()
            .find(|node| {
                node.fname() == protocol::get_field_by_symbol("sfModifiedNode")
                    && node.get_field_h256(protocol::get_field_by_symbol("sfLedgerIndex"))
                        == modify_keylet.key
            })
            .expect("modified delta needs a ModifiedNode");
        assert_eq!(
            modified_node.get_field_h256(protocol::get_field_by_symbol("sfPreviousTxnID")),
            previous_transaction,
            "threadItem records the prior transaction on the affected node"
        );
        assert_eq!(
            modified_node.get_field_u32(protocol::get_field_by_symbol("sfPreviousTxnLgrSeq")),
            41,
            "threadItem records the prior ledger sequence on the affected node"
        );
        assert!(nodes.iter().any(|node| {
            node.fname() == protocol::get_field_by_symbol("sfDeletedNode")
                && node.get_field_h256(protocol::get_field_by_symbol("sfLedgerIndex"))
                    == erase_keylet.key
        }));
        assert!(nodes.iter().any(|node| {
            node.fname() == protocol::get_field_by_symbol("sfCreatedNode")
                && node.get_field_h256(protocol::get_field_by_symbol("sfLedgerIndex"))
                    == insert_keylet.key
        }));
    }

    #[test]
    fn no_op_modify_is_neither_threaded_nor_reported() {
        let keylet = Keylet::new(LedgerEntryType::RippleState, Uint256::from_u64(0xB6E1));
        let mut original = STLedgerEntry::new(keylet);
        let prior_tx = Uint256::from_u64(0x347A);
        original.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
        original.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 41);

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(original.clone()))
            .expect("seed trust line");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();
        let current_tx = Uint256::from_u64(0xA5EA);

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            delta
                .raw_replace(Arc::new(original.clone()))
                .expect("stage redundant TrustSet result");
            let metadata = delta
                .to_tx_meta(current_tx, 42, None, &rules)
                .expect("metadata");
            delta
                .apply_with_tx_thread(current_tx, 42, &rules)
                .expect("commit");
            metadata
        };

        assert!(metadata.get_nodes().is_empty());
        let retained = ReadView::read(&parent, keylet)
            .expect("read trust line")
            .expect("trust line remains");
        assert_eq!(
            retained.as_ref(),
            &original,
            "redundant TrustSet must retain the existing transaction thread"
        );
    }

    #[test]
    fn ticket_count_creation_emits_canonical_empty_previous_fields() {
        // TicketCreate materializes an optional sfTicketCount slot. rippled's
        // typed SLE loop selects the original NotPresent slot, so metadata
        // contains an empty PreviousFields object rather than omitting it.
        let account_keylet = Keylet::new(LedgerEntryType::AccountRoot, Uint256::from_u64(0xD4E087));
        let original = STLedgerEntry::new(account_keylet);
        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(original.clone()))
            .expect("seed account root without TicketCount");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            let mut updated = original.clone_as_object();
            updated.set_field_u32(protocol::get_field_by_symbol("sfTicketCount"), 140);
            delta
                .raw_replace(Arc::new(STLedgerEntry::from_stobject(
                    updated,
                    account_keylet.key,
                )))
                .expect("stage TicketCount creation");
            delta
                .to_tx_meta(Uint256::from_u64(0xB61E52), 20_111_099, None, &rules)
                .expect("build TicketCreate metadata")
        };

        let modified = metadata
            .get_nodes()
            .iter()
            .find(|node| {
                node.fname() == protocol::get_field_by_symbol("sfModifiedNode")
                    && node.get_field_h256(protocol::get_field_by_symbol("sfLedgerIndex"))
                        == account_keylet.key
            })
            .expect("account root needs a ModifiedNode");
        let previous_field = protocol::get_field_by_symbol("sfPreviousFields");
        assert!(
            modified.is_field_present(previous_field),
            "selected NotPresent slot must still materialize PreviousFields"
        );
        let previous = modified.get_field_object(previous_field);
        assert!(
            previous
                .iter()
                .all(|field| field.stype() == SerializedTypeId::NotPresent),
            "the selected absent TicketCount slot must serialize as an empty object"
        );
        assert!(
            protocol::serialize_blob(modified)
                .windows(2)
                .any(|bytes| bytes == [0xE6, 0xE1]),
            "affected-node serialization needs canonical empty PreviousFields E6E1"
        );
    }

    #[test]
    fn directed_nftoken_create_offer_threads_destination_account_and_metadata() {
        // A created NFTokenOffer carries sfDestination. rippled threadOwners
        // therefore adds a thread-only modification for that AccountRoot even
        // though the transactor does not otherwise mutate the destination.
        let destination = protocol::AccountID::from_array([0x22; 20]);
        let destination_keylet = account_keylet(Uint160::from_void(destination.data()));
        let offer_keylet = Keylet::new(LedgerEntryType::NFTokenOffer, Uint256::from_u64(0x0FFE12));
        let prior_tx = Uint256::from_u64(0xBEEF);
        let current_tx = Uint256::from_u64(0xA11CE);
        let ledger_seq = 20_115_081;

        let mut destination_root = STLedgerEntry::new(destination_keylet);
        destination_root.set_account_id(get_field_by_symbol("sfAccount"), destination);
        destination_root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
        destination_root.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(destination_root))
            .expect("seed directed-offer destination");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            let mut offer = STLedgerEntry::new(offer_keylet);
            offer.set_account_id(get_field_by_symbol("sfDestination"), destination);
            delta.insert(Arc::new(offer)).expect("stage directed offer");

            let metadata = delta
                .to_tx_meta(current_tx, ledger_seq, None, &rules)
                .expect("build directed-offer metadata");
            delta
                .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                .expect("commit directed offer with owner threads");
            metadata
        };

        let destination_after = parent
            .read(destination_keylet)
            .expect("read destination after commit")
            .expect("destination remains present");
        assert_eq!(
            destination_after.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
            current_tx
        );
        assert_eq!(
            destination_after.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
            ledger_seq
        );

        let destination_meta = metadata
            .get_nodes()
            .iter()
            .find(|node| {
                node.fname() == get_field_by_symbol("sfModifiedNode")
                    && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                        == destination_keylet.key
            })
            .expect("destination needs a supplemental ModifiedNode");
        assert_eq!(
            destination_meta.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
            prior_tx
        );
        assert_eq!(
            destination_meta.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
            ledger_seq - 1
        );
        assert!(
            !destination_meta.is_field_present(get_field_by_symbol("sfFinalFields")),
            "rippled newMod owner threads contain only the prior transaction thread"
        );
        assert!(
            !destination_meta.is_field_present(get_field_by_symbol("sfPreviousFields")),
            "thread-only owner metadata must not synthesize PreviousFields"
        );
        assert_eq!(
            destination_meta.iter().count(),
            4,
            "supplemental ModifiedNode must contain only its type, index, and prior thread pair"
        );
        assert_eq!(
            metadata
                .get_nodes()
                .iter()
                .next()
                .expect("created offer metadata node")
                .get_field_h256(get_field_by_symbol("sfLedgerIndex")),
            offer_keylet.key,
            "rippled registers the CreatedNode before supplemental owner metadata"
        );
    }

    #[test]
    fn owner_thread_without_prior_transaction_updates_state_without_empty_metadata_node() {
        // threadItem mutates a never-threaded AccountRoot, but rippled only
        // creates a supplemental ModifiedNode when prevTxID is non-zero.
        let destination = protocol::AccountID::from_array([0x24; 20]);
        let destination_keylet = account_keylet(Uint160::from_void(destination.data()));
        let offer_keylet = Keylet::new(LedgerEntryType::NFTokenOffer, Uint256::from_u64(0x2400));
        let current_tx = Uint256::from_u64(0x2401);

        let mut destination_root = STLedgerEntry::new(destination_keylet);
        destination_root.set_account_id(get_field_by_symbol("sfAccount"), destination);
        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(destination_root))
            .expect("seed never-threaded destination");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            let mut offer = STLedgerEntry::new(offer_keylet);
            offer.set_account_id(get_field_by_symbol("sfDestination"), destination);
            delta.insert(Arc::new(offer)).expect("stage directed offer");
            let metadata = delta
                .to_tx_meta(current_tx, 42, None, &rules)
                .expect("build metadata");
            delta
                .apply_with_tx_thread(current_tx, 42, &rules)
                .expect("commit owner thread");
            metadata
        };

        assert!(!metadata.get_nodes().iter().any(|node| {
            node.fname() == get_field_by_symbol("sfModifiedNode")
                && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                    == destination_keylet.key
        }));
        let destination_after = parent
            .read(destination_keylet)
            .expect("read destination")
            .expect("destination remains present");
        assert_eq!(
            destination_after.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
            current_tx
        );
    }

    #[test]
    fn multiple_created_entries_reuse_one_supplemental_owner_thread() {
        // ApplyStateTable's newMod map is shared across the full ordered pass.
        // Two created objects for one otherwise-untouched owner must update
        // that AccountRoot once and emit one supplemental ModifiedNode.
        let destination = protocol::AccountID::from_array([0x25; 20]);
        let destination_keylet = account_keylet(Uint160::from_void(destination.data()));
        let prior_tx = Uint256::from_u64(0x2500);
        let current_tx = Uint256::from_u64(0x2501);
        let ledger_seq = 20_115_081;
        let mut destination_root = STLedgerEntry::new(destination_keylet);
        destination_root.set_account_id(get_field_by_symbol("sfAccount"), destination);
        destination_root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
        destination_root.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(destination_root))
            .expect("seed shared destination");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();
        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            for key in [Uint256::zero(), Uint256::from_array([0xFF; 32])] {
                let mut offer = STLedgerEntry::new(Keylet::new(LedgerEntryType::NFTokenOffer, key));
                offer.set_account_id(get_field_by_symbol("sfDestination"), destination);
                delta.insert(Arc::new(offer)).expect("stage directed offer");
            }
            let metadata = delta
                .to_tx_meta(current_tx, ledger_seq, None, &rules)
                .expect("build shared-owner metadata");
            delta
                .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                .expect("commit shared owner thread");
            metadata
        };

        assert_eq!(
            metadata
                .get_nodes()
                .iter()
                .filter(|node| {
                    node.fname() == get_field_by_symbol("sfModifiedNode")
                        && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                            == destination_keylet.key
                })
                .count(),
            1
        );
        assert_eq!(
            metadata
                .get_nodes()
                .iter()
                .map(|node| node.get_field_h256(get_field_by_symbol("sfLedgerIndex")))
                .collect::<Vec<_>>(),
            vec![
                Uint256::zero(),
                destination_keylet.key,
                Uint256::from_array([0xFF; 32]),
            ],
            "newMod is inserted once between the two ordered CreatedNodes"
        );
        assert_eq!(
            parent
                .read(destination_keylet)
                .expect("read shared destination")
                .expect("shared destination remains")
                .get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
            current_tx
        );
    }

    #[test]
    fn owner_thread_revives_unchanged_modify_in_either_item_order() {
        // ApplyStateTable::getForMod aliases an existing Modify entry. The
        // state result is order-independent, but rippled's metadata is not:
        // an owner threaded before its own map position later runs through the
        // ordinary ModifiedNode branch and gains FinalFields; an owner whose
        // unchanged item was already skipped remains thread-only.
        let destination = protocol::AccountID::from_array([0x33; 20]);
        let destination_keylet = account_keylet(Uint160::from_void(destination.data()));
        for offer_key in [Uint256::zero(), Uint256::from_array([0xFF; 32])] {
            let offer_keylet = Keylet::new(LedgerEntryType::NFTokenOffer, offer_key);
            let prior_tx = Uint256::from_u64(0xCAFE);
            let current_tx = Uint256::from_u64(0xF00D);
            let ledger_seq = 20_115_082;

            let mut destination_root = STLedgerEntry::new(destination_keylet);
            destination_root.set_account_id(get_field_by_symbol("sfAccount"), destination);
            destination_root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
            destination_root
                .set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);

            let mut base = Ledger::new(LedgerHeader::default(), false);
            base.raw_insert(Arc::new(destination_root.clone()))
                .expect("seed unchanged destination");
            let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
            let rules = parent.rules();

            let metadata = {
                let mut delta = FlowSandbox::new(&mut parent);
                delta
                    .raw_replace(Arc::new(destination_root.clone()))
                    .expect("stage unchanged Modify");
                let mut offer = STLedgerEntry::new(offer_keylet);
                offer.set_account_id(get_field_by_symbol("sfDestination"), destination);
                delta
                    .insert(Arc::new(offer))
                    .expect("stage owner-bearing entry");

                let metadata = delta
                    .to_tx_meta(current_tx, ledger_seq, None, &rules)
                    .expect("owner threading must revive unchanged Modify metadata");
                delta
                    .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                    .expect("owner threading must commit unchanged Modify");
                metadata
            };

            let matching_nodes = metadata
                .get_nodes()
                .iter()
                .filter(|node| {
                    node.fname() == get_field_by_symbol("sfModifiedNode")
                        && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                            == destination_keylet.key
                })
                .collect::<Vec<_>>();
            assert_eq!(matching_nodes.len(), 1, "owner thread must be deduplicated");
            assert_eq!(
                matching_nodes[0].get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                prior_tx
            );
            let source_precedes_owner = offer_key < destination_keylet.key;
            assert_eq!(
                matching_nodes[0].is_field_present(get_field_by_symbol("sfFinalFields")),
                source_precedes_owner,
                "FinalFields presence must follow rippled's ordered items_ mutation"
            );
            if source_precedes_owner {
                assert_eq!(
                    matching_nodes[0]
                        .get_field_object(get_field_by_symbol("sfFinalFields"))
                        .get_account_id(get_field_by_symbol("sfAccount")),
                    destination,
                    "revived normal ModifiedNode must carry canonical AccountRoot FinalFields"
                );
            }
            assert_eq!(
                matching_nodes[0].iter().count(),
                if source_precedes_owner { 5 } else { 4 },
                "owner metadata must contain exactly the rippled fields for its ordering case"
            );

            let destination_after = parent
                .read(destination_keylet)
                .expect("read revived destination")
                .expect("destination remains present");
            assert_eq!(
                destination_after.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                current_tx
            );
            assert_eq!(
                destination_after.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
                ledger_seq
            );
        }
    }

    #[test]
    fn erased_owner_source_revives_unchanged_modify_in_either_item_order() {
        // DeletedNode runs threadOwners at the same point as CreatedNode.
        // Cover both sides of the ordered-map alias case independently so an
        // EscrowFinish/PayChannelClaim deletion cannot regress while fixing a
        // TrustSet-style supplemental owner.
        let destination = protocol::AccountID::from_array([0x35; 20]);
        let destination_keylet = account_keylet(Uint160::from_void(destination.data()));
        for escrow_key in [Uint256::from_u64(1), Uint256::from_array([0xFF; 32])] {
            let escrow_keylet = Keylet::new(LedgerEntryType::Escrow, escrow_key);
            let prior_tx = Uint256::from_u64(0x3500);
            let current_tx = Uint256::from_u64(0x3501);
            let ledger_seq = 20_115_083;
            let mut destination_root = STLedgerEntry::new(destination_keylet);
            destination_root.set_account_id(get_field_by_symbol("sfAccount"), destination);
            destination_root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
            destination_root
                .set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);
            let mut escrow = STLedgerEntry::new(escrow_keylet);
            escrow.set_account_id(get_field_by_symbol("sfDestination"), destination);

            let mut base = Ledger::new(LedgerHeader::default(), false);
            base.raw_insert(Arc::new(destination_root.clone()))
                .expect("seed unchanged destination");
            base.raw_insert(Arc::new(escrow.clone()))
                .expect("seed erased owner source");
            let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
            let rules = parent.rules();
            let metadata = {
                let mut delta = FlowSandbox::new(&mut parent);
                delta
                    .raw_replace(Arc::new(destination_root.clone()))
                    .expect("stage unchanged Modify");
                delta
                    .raw_erase(Arc::new(escrow))
                    .expect("stage Escrow erase");
                let metadata = delta
                    .to_tx_meta(current_tx, ledger_seq, None, &rules)
                    .expect("build erased-owner metadata");
                delta
                    .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                    .expect("commit erased-owner thread");
                metadata
            };

            let destination_node = metadata
                .get_nodes()
                .iter()
                .find(|node| {
                    node.fname() == get_field_by_symbol("sfModifiedNode")
                        && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                            == destination_keylet.key
                })
                .expect("erased source must thread destination");
            assert_eq!(
                destination_node.is_field_present(get_field_by_symbol("sfFinalFields")),
                escrow_key < destination_keylet.key,
                "DeletedNode owner metadata must retain rippled's ordered alias behavior"
            );
            if escrow_key < destination_keylet.key {
                assert_eq!(
                    destination_node
                        .get_field_object(get_field_by_symbol("sfFinalFields"))
                        .get_account_id(get_field_by_symbol("sfAccount")),
                    destination
                );
            }
            assert_eq!(
                parent
                    .read(destination_keylet)
                    .expect("read threaded destination")
                    .expect("destination remains")
                    .get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                current_tx
            );
        }
    }

    #[test]
    fn deleted_ripple_state_threads_both_issuer_accounts_and_metadata() {
        let low = protocol::AccountID::from_array([0x41; 20]);
        let high = protocol::AccountID::from_array([0x42; 20]);
        let low_keylet = account_keylet(Uint160::from_void(low.data()));
        let high_keylet = account_keylet(Uint160::from_void(high.data()));
        let line_keylet = Keylet::new(LedgerEntryType::RippleState, Uint256::from_u64(0x5157));
        let prior_tx = Uint256::from_u64(0x1111);
        let current_tx = Uint256::from_u64(0x2222);
        let ledger_seq = 20_115_083;

        let make_account = |keylet: Keylet, account: protocol::AccountID| {
            let mut root = STLedgerEntry::new(keylet);
            root.set_account_id(get_field_by_symbol("sfAccount"), account);
            root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
            root.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);
            root
        };
        let currency = protocol::currency_from_string("USD");
        let mut line = STLedgerEntry::new(line_keylet);
        line.set_field_amount(
            get_field_by_symbol("sfLowLimit"),
            protocol::STAmount::from_iou_amount(
                get_field_by_symbol("sfLowLimit"),
                protocol::IOUAmount::new(),
                protocol::Issue::new(currency, low),
            ),
        );
        line.set_field_amount(
            get_field_by_symbol("sfHighLimit"),
            protocol::STAmount::from_iou_amount(
                get_field_by_symbol("sfHighLimit"),
                protocol::IOUAmount::new(),
                protocol::Issue::new(currency, high),
            ),
        );

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(make_account(low_keylet, low)))
            .expect("seed low issuer");
        base.raw_insert(Arc::new(make_account(high_keylet, high)))
            .expect("seed high issuer");
        base.raw_insert(Arc::new(line.clone()))
            .expect("seed trust line");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            delta.raw_erase(Arc::new(line)).expect("erase trust line");
            let metadata = delta
                .to_tx_meta(current_tx, ledger_seq, None, &rules)
                .expect("build trust-line deletion metadata");
            delta
                .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                .expect("commit trust-line deletion");
            metadata
        };

        for owner_keylet in [low_keylet, high_keylet] {
            let owner = parent
                .read(owner_keylet)
                .expect("read threaded issuer")
                .expect("issuer remains present");
            assert_eq!(
                owner.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                current_tx
            );
            assert_eq!(
                owner.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
                ledger_seq
            );
            let nodes = metadata
                .get_nodes()
                .iter()
                .filter(|node| {
                    node.fname() == get_field_by_symbol("sfModifiedNode")
                        && node.get_field_h256(get_field_by_symbol("sfLedgerIndex"))
                            == owner_keylet.key
                })
                .collect::<Vec<_>>();
            assert_eq!(nodes.len(), 1, "each issuer needs one supplemental node");
            assert_eq!(
                nodes[0].get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                prior_tx
            );
            assert_eq!(
                nodes[0].get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
                ledger_seq - 1
            );
        }
        assert_eq!(
            metadata
                .get_nodes()
                .iter()
                .next()
                .expect("deleted trust-line metadata node")
                .get_field_h256(get_field_by_symbol("sfLedgerIndex")),
            line_keylet.key,
            "rippled registers the DeletedNode before supplemental owner metadata"
        );
        assert!(
            !parent.exists(line_keylet).expect("read erased trust line"),
            "trust line must remain erased"
        );
    }

    #[test]
    fn erased_generic_entry_threads_account_and_destination_and_skips_absent_owner() {
        let account = protocol::AccountID::from_array([0x51; 20]);
        let destination = protocol::AccountID::from_array([0x52; 20]);
        let account_key = account_keylet(Uint160::from_void(account.data()));
        let destination_key = account_keylet(Uint160::from_void(destination.data()));
        let escrow_key = Keylet::new(LedgerEntryType::Escrow, Uint256::from_u64(0xE5C20));
        let prior_tx = Uint256::from_u64(0x3333);
        let current_tx = Uint256::from_u64(0x4444);
        let ledger_seq = 20_115_084;

        let make_account = |keylet: Keylet, id: protocol::AccountID| {
            let mut root = STLedgerEntry::new(keylet);
            root.set_account_id(get_field_by_symbol("sfAccount"), id);
            root.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), prior_tx);
            root.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq - 1);
            root
        };
        let mut escrow = STLedgerEntry::new(escrow_key);
        escrow.set_account_id(get_field_by_symbol("sfAccount"), account);
        escrow.set_account_id(get_field_by_symbol("sfDestination"), destination);

        let mut base = Ledger::new(LedgerHeader::default(), false);
        base.raw_insert(Arc::new(make_account(account_key, account)))
            .expect("seed escrow owner");
        base.raw_insert(Arc::new(make_account(destination_key, destination)))
            .expect("seed escrow destination");
        base.raw_insert(Arc::new(escrow.clone()))
            .expect("seed escrow");
        let mut parent = Sandbox::new(Arc::new(base), ApplyFlags::default());
        let rules = parent.rules();

        let metadata = {
            let mut delta = FlowSandbox::new(&mut parent);
            delta.raw_erase(Arc::new(escrow)).expect("erase escrow");
            let metadata = delta
                .to_tx_meta(current_tx, ledger_seq, None, &rules)
                .expect("build escrow deletion metadata");
            delta
                .apply_with_tx_thread(current_tx, ledger_seq, &rules)
                .expect("commit escrow deletion");
            metadata
        };

        for owner_keylet in [account_key, destination_key] {
            let owner = parent
                .read(owner_keylet)
                .expect("read threaded escrow party")
                .expect("escrow party remains present");
            assert_eq!(
                owner.get_field_h256(get_field_by_symbol("sfPreviousTxnID")),
                current_tx
            );
            assert_eq!(
                owner.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
                ledger_seq
            );
            assert!(metadata.get_nodes().iter().any(|node| {
                node.fname() == get_field_by_symbol("sfModifiedNode")
                    && node.get_field_h256(get_field_by_symbol("sfLedgerIndex")) == owner_keylet.key
                    && node.get_field_h256(get_field_by_symbol("sfPreviousTxnID")) == prior_tx
                    && node.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"))
                        == ledger_seq - 1
            }));
        }

        // A missing referenced account matches getForMod's nullptr path: the
        // existing party still threads, while no synthetic account or metadata
        // node is created for the absent destination.
        let missing = protocol::AccountID::from_array([0x53; 20]);
        let missing_key = account_keylet(Uint160::from_void(missing.data()));
        let missing_current_tx = Uint256::from_u64(0x5555);
        let missing_escrow_key = Keylet::new(LedgerEntryType::Escrow, Uint256::from_u64(0xE5C21));
        let mut missing_escrow = STLedgerEntry::new(missing_escrow_key);
        missing_escrow.set_account_id(get_field_by_symbol("sfAccount"), account);
        missing_escrow.set_account_id(get_field_by_symbol("sfDestination"), missing);
        parent
            .insert(Arc::new(missing_escrow.clone()))
            .expect("seed escrow with missing destination");

        let missing_meta = {
            let mut delta = FlowSandbox::new(&mut parent);
            delta
                .raw_erase(Arc::new(missing_escrow))
                .expect("erase escrow with missing destination");
            let metadata = delta
                .to_tx_meta(missing_current_tx, ledger_seq + 1, None, &rules)
                .expect("missing destination is non-fatal");
            delta
                .apply_with_tx_thread(missing_current_tx, ledger_seq + 1, &rules)
                .expect("commit with missing destination");
            metadata
        };
        assert!(!parent.exists(missing_key).expect("missing stays absent"));
        assert!(!missing_meta.get_nodes().iter().any(|node| {
            node.get_field_h256(get_field_by_symbol("sfLedgerIndex")) == missing_key.key
        }));
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
