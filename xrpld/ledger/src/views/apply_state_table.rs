//! Rust port of `xrpl::detail::ApplyStateTable` from `xrpl/ledger/detail/ApplyStateTable.h`.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{Keylet, Rules, STLedgerEntry, XRPAmount};

use crate::raw_view::RawView;
use crate::read_view::{ReadView, ViewError};

fn invariant_error(message: &str) -> ViewError {
    ViewError::Conversion(message.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Cache,
    Erase,
    Insert,
    Modify,
}

#[derive(Debug, Clone)]
pub struct StateEntry {
    pub action: Action,
    pub sle: Arc<STLedgerEntry>,
}

#[derive(Debug)]
pub struct ApplyStateTable {
    items: BTreeMap<Uint256, StateEntry>,
    drops_destroyed: XRPAmount,
}

impl Default for ApplyStateTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplyStateTable {
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
            drops_destroyed: XRPAmount::new(),
        }
    }

    pub fn size(&self) -> usize {
        self.items.len()
    }

    pub fn drops_destroyed(&self) -> XRPAmount {
        self.drops_destroyed
    }

    /// Debug: return a summary of all modifications (inserts, modifies, erases).
    pub fn modification_summary(&self) -> String {
        let mut inserts = 0u32;
        let mut modifies = 0u32;
        let mut erases = 0u32;
        let mut details = Vec::new();
        for (key, entry) in &self.items {
            match entry.action {
                Action::Insert => {
                    inserts += 1;
                    details.push(format!(
                        "I:{:02x}{:02x}{:02x}{:02x}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3]
                    ));
                }
                Action::Modify => {
                    modifies += 1;
                    details.push(format!(
                        "M:{:02x}{:02x}{:02x}{:02x}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3]
                    ));
                }
                Action::Erase => {
                    erases += 1;
                    details.push(format!(
                        "E:{:02x}{:02x}{:02x}{:02x}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3]
                    ));
                }
                Action::Cache => {} // read-only, not a modification
            }
        }
        format!(
            "i={} m={} e={} fee={} keys=[{}]",
            inserts,
            modifies,
            erases,
            self.drops_destroyed.drops(),
            details.join(",")
        )
    }

    /// Full sync debug detail for every touched ledger object.
    pub fn modification_debug_lines(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|(key, entry)| {
                let action = match entry.action {
                    Action::Cache => return None,
                    Action::Erase => "erase",
                    Action::Insert => "insert",
                    Action::Modify => "modify",
                };
                let payload = entry.sle.get_serializer().data().to_vec();
                Some(format!(
                    "action={} key={} type={:?} bytes={} first16={} hex={}",
                    action,
                    key,
                    entry.sle.get_type(),
                    payload.len(),
                    payload
                        .iter()
                        .take(16)
                        .map(|byte| format!("{:02x}", byte))
                        .collect::<Vec<_>>()
                        .join(""),
                    payload
                        .iter()
                        .map(|byte| format!("{:02x}", byte))
                        .collect::<Vec<_>>()
                        .join("")
                ))
            })
            .collect()
    }

    pub fn destroy_xrp(&mut self, fee: XRPAmount) {
        self.drops_destroyed += fee;
    }

    pub fn succ(
        &self,
        base: &dyn ReadView,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, ViewError> {
        let mut next = Some(key);
        // Find base successor that is not also deleted in our list
        loop {
            next = base.succ(next.unwrap(), last)?;
            let Some(n) = next else { break };
            if let Some(entry) = self.items.get(&n)
                && entry.action == Action::Erase
            {
                continue;
            }
            break;
        }

        // Find non-deleted successor in our list
        let local_next = self
            .items
            .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
            .find(|(_, entry)| entry.action != Action::Erase)
            .map(|(k, _)| *k);

        // Pick the smaller of base result and local result
        let result = match (next, local_next) {
            (Some(n), Some(ln)) => Some(n.min(ln)),
            (Some(n), None) => Some(n),
            (None, Some(ln)) => Some(ln),
            (None, None) => None,
        };

        // Apply the last-key filter
        if let Some(n) = result
            && let Some(l) = last
            && n >= l
        {
            return Ok(None);
        }

        Ok(result)
    }

    pub fn exists(&self, base: &dyn ReadView, k: Keylet) -> Result<bool, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            return Ok(entry.action != Action::Erase);
        }
        base.exists(k)
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
            return Ok(Some(entry.sle.clone()));
        }
        base.read(k)
    }

    pub fn peek(
        &mut self,
        base: &dyn ReadView,
        k: Keylet,
    ) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        if let Some(entry) = self.items.get(&k.key) {
            if entry.action == Action::Erase {
                return Ok(None);
            }
            return Ok(Some(entry.sle.clone()));
        }

        let sle = base.read(k)?;
        if let Some(sle) = &sle {
            // Cache it
            self.items.insert(
                k.key,
                StateEntry {
                    action: Action::Cache,
                    sle: sle.clone(),
                },
            );
        }
        Ok(sle)
    }

    pub fn insert(
        &mut self,
        _base: &dyn ReadView,
        sle: Arc<STLedgerEntry>,
    ) -> Result<(), ViewError> {
        let key = *sle.key();
        if let Some(entry) = self.items.get_mut(&key) {
            match entry.action {
                Action::Cache => {
                    return Err(invariant_error("ApplyStateTable::insert: already cached"));
                }
                Action::Insert => {
                    return Err(invariant_error("ApplyStateTable::insert: already inserted"));
                }
                Action::Modify => {
                    return Err(invariant_error("ApplyStateTable::insert: already modified"));
                }
                Action::Erase => {
                    entry.sle = sle;
                    entry.action = Action::Modify;
                }
            }
            return Ok(());
        }

        self.items.insert(
            key,
            StateEntry {
                action: Action::Insert,
                sle,
            },
        );
        tracing::trace!(target: "ledger", key = %key, action = "insert", "State table mutation");
        Ok(())
    }

    pub fn replace(
        &mut self,
        _base: &dyn ReadView,
        sle: Arc<STLedgerEntry>,
    ) -> Result<(), ViewError> {
        let key = *sle.key();
        if let Some(entry) = self.items.get_mut(&key) {
            match entry.action {
                Action::Erase => {
                    return Err(invariant_error("ApplyStateTable::replace: already erased"));
                }
                Action::Cache => {
                    entry.action = Action::Modify;
                }
                Action::Insert | Action::Modify => {}
            }
            entry.sle = sle;
            return Ok(());
        }

        self.items.insert(
            key,
            StateEntry {
                action: Action::Modify,
                sle,
            },
        );
        tracing::trace!(target: "ledger", key = %key, action = "modify", "State table mutation");
        Ok(())
    }

    pub fn update(
        &mut self,
        _base: &dyn ReadView,
        sle: Arc<STLedgerEntry>,
    ) -> Result<(), ViewError> {
        let key = *sle.key();
        let Some(entry) = self.items.get_mut(&key) else {
            return Err(invariant_error("ApplyStateTable::update: missing key"));
        };

        match entry.action {
            Action::Erase => Err(invariant_error("ApplyStateTable::update: erased")),
            Action::Cache => {
                entry.action = Action::Modify;
                entry.sle = sle;
                Ok(())
            }
            Action::Insert | Action::Modify => {
                entry.sle = sle;
                Ok(())
            }
        }
    }

    pub fn erase(
        &mut self,
        _base: &dyn ReadView,
        sle: Arc<STLedgerEntry>,
    ) -> Result<(), ViewError> {
        let key = *sle.key();
        let Some(entry) = self.items.get_mut(&key) else {
            return Err(invariant_error("ApplyStateTable::erase: missing key"));
        };

        match entry.action {
            Action::Erase => Err(invariant_error("ApplyStateTable::erase: double erase")),
            Action::Insert => {
                self.items.remove(&key);
                Ok(())
            }
            Action::Cache | Action::Modify => {
                entry.action = Action::Erase;
                tracing::trace!(target: "ledger", key = %key, action = "erase", "State table mutation");
                entry.sle = sle;
                Ok(())
            }
        }
    }

    pub fn apply(&self, to: &mut dyn RawView) -> Result<(), ViewError> {
        // Collect all state map operations into a batch to apply using a single
        // MutableTree. This prevents MissingNode errors when sequential mutations
        // create new inner nodes that subsequent mutations can't find in NuDB.
        let mut batch_ops: Vec<(crate::StateBatchOp, basics::base_uint::Uint256, Vec<u8>)> =
            Vec::new();

        for (key, entry) in &self.items {
            match entry.action {
                Action::Erase => {
                    tracing::debug!(target: "ledger",                        "[sandbox_apply] ERASE key={:02x}{:02x}{:02x}{:02x} sle_type={:?}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3],
                        entry.sle.get_type(),
                    );
                    batch_ops.push((crate::StateBatchOp::Delete, *key, Vec::new()));
                }
                Action::Insert => {
                    tracing::debug!(target: "ledger",                        "[sandbox_apply] INSERT key={:02x}{:02x}{:02x}{:02x} sle_type={:?}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3],
                        entry.sle.get_type(),
                    );
                    let payload = entry.sle.get_serializer().data().to_vec();
                    batch_ops.push((crate::StateBatchOp::Insert, *key, payload));
                }
                Action::Modify => {
                    tracing::debug!(target: "ledger",                        "[sandbox_apply] MODIFY key={:02x}{:02x}{:02x}{:02x} sle_type={:?}",
                        key.data()[0],
                        key.data()[1],
                        key.data()[2],
                        key.data()[3],
                        entry.sle.get_type(),
                    );
                    let payload = entry.sle.get_serializer().data().to_vec();
                    batch_ops.push((crate::StateBatchOp::Update, *key, payload));
                }
                Action::Cache => {}
            }
        }

        // Apply all state map operations in a single MutableTree session
        to.raw_apply_batch(&batch_ops)?;

        if self.drops_destroyed.drops() != 0 {
            tracing::debug!(target: "ledger",                "[sandbox_apply] DESTROY_XRP drops={}",
                self.drops_destroyed.drops()
            );
        }
        to.raw_destroy_xrp(self.drops_destroyed)?;
        Ok(())
    }

    /// Apply closed-ledger transaction mutations after threading modified
    /// ledger entries with the current transaction id and ledger sequence.
    ///
    /// calls `threadItem(meta, curNode)` before `apply(to)`, and
    /// `STLedgerEntry::thread(...)` writes `sfPreviousTxnID` and
    /// `sfPreviousTxnLgrSeq` into the SLE that is later raw-applied.
    pub fn apply_with_tx_thread(
        &self,
        to: &mut dyn RawView,
        tx_id: basics::base_uint::Uint256,
        ledger_seq: u32,
        rules: &Rules,
    ) -> Result<(), ViewError> {
        let mut batch_ops: Vec<(crate::StateBatchOp, basics::base_uint::Uint256, Vec<u8>)> =
            Vec::new();

        for (key, entry) in &self.items {
            match entry.action {
                Action::Erase => {
                    batch_ops.push((crate::StateBatchOp::Delete, *key, Vec::new()));
                }
                Action::Insert | Action::Modify => {
                    let payload = threaded_payload(entry.sle.as_ref(), tx_id, ledger_seq, rules);
                    let op = if entry.action == Action::Insert {
                        crate::StateBatchOp::Insert
                    } else {
                        crate::StateBatchOp::Update
                    };
                    if crate::full_sync_debug_enabled() {
                        tracing::debug!(target: "ledger",                            "[full_debug][tx_thread_payload] ledger_seq={} txid={} op={:?} key={} type={:?} bytes={} first16={} hex={}",
                            ledger_seq,
                            tx_id,
                            op,
                            key,
                            entry.sle.get_type(),
                            payload.len(),
                            payload
                                .iter()
                                .take(16)
                                .map(|byte| format!("{:02x}", byte))
                                .collect::<Vec<_>>()
                                .join(""),
                            payload
                                .iter()
                                .map(|byte| format!("{:02x}", byte))
                                .collect::<Vec<_>>()
                                .join("")
                        );
                    }
                    batch_ops.push((op, *key, payload));
                }
                Action::Cache => {}
            }
        }

        to.raw_apply_batch(&batch_ops)?;
        to.raw_destroy_xrp(self.drops_destroyed)?;
        Ok(())
    }
    /// Generate simulation metadata in the AffectedNodes format.
    pub fn to_simulation_metadata(&self) -> Vec<protocol::JsonValue> {
        use protocol::JsonValue;
        let mut nodes = Vec::new();
        for (key, entry) in &self.items {
            let type_str = JsonValue::String(format!("{:?}", entry.sle.get_type()));
            let index_str = JsonValue::String(format!("{key}"));
            match entry.action {
                Action::Insert => {
                    let mut fields = std::collections::BTreeMap::new();
                    fields.insert("LedgerEntryType".to_owned(), type_str);
                    fields.insert("LedgerIndex".to_owned(), index_str);
                    let mut node = std::collections::BTreeMap::new();
                    node.insert("CreatedNode".to_owned(), JsonValue::Object(fields));
                    nodes.push(JsonValue::Object(node));
                }
                Action::Modify => {
                    let mut fields = std::collections::BTreeMap::new();
                    fields.insert("LedgerEntryType".to_owned(), type_str);
                    fields.insert("LedgerIndex".to_owned(), index_str);
                    let mut node = std::collections::BTreeMap::new();
                    node.insert("ModifiedNode".to_owned(), JsonValue::Object(fields));
                    nodes.push(JsonValue::Object(node));
                }
                Action::Erase => {
                    let mut fields = std::collections::BTreeMap::new();
                    fields.insert("LedgerEntryType".to_owned(), type_str);
                    fields.insert("LedgerIndex".to_owned(), index_str);
                    let mut node = std::collections::BTreeMap::new();
                    node.insert("DeletedNode".to_owned(), JsonValue::Object(fields));
                    nodes.push(JsonValue::Object(node));
                }
                Action::Cache => {}
            }
        }
        nodes
    }
}

fn threaded_payload(
    sle: &STLedgerEntry,
    tx_id: basics::base_uint::Uint256,
    ledger_seq: u32,
    rules: &Rules,
) -> Vec<u8> {
    if !sle.is_threaded_type(rules) {
        return sle.get_serializer().data().to_vec();
    }

    let mut threaded = sle.clone();
    let mut previous_txn_id = basics::base_uint::Uint256::zero();
    let mut previous_ledger_seq = 0;
    let _ = threaded.thread(
        tx_id,
        ledger_seq,
        &mut previous_txn_id,
        &mut previous_ledger_seq,
    );
    threaded.get_serializer().data().to_vec()
}
