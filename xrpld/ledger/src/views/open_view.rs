//! Rust `OpenView` port aligned to the the reference implementation `OpenView` +
//! `detail::RawStateTable` contract.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{
    Keylet, LedgerEntryType, Rules, STLedgerEntry, STObject, STTx, Serializer, XRPAmount,
};

use crate::raw_view::{RawView, TxsRawView};
use crate::read_view::{ReadView, ReadViewTx, ViewError};
use crate::{Fees, LedgerHeader, RawStateTable};

#[derive(Debug, Clone)]
struct TxEntry {
    txn: Arc<Serializer>,
    metadata: Option<Arc<Serializer>>,
}

#[derive(Debug)]
pub struct OpenView<B> {
    base: Arc<B>,
    rules: Rules,
    header: LedgerHeader,
    items: RawStateTable,
    txs: BTreeMap<Uint256, TxEntry>,
    base_tx_count: usize,
    open: bool,
}

impl<B> Clone for OpenView<B>
where
    B: ReadView,
{
    fn clone(&self) -> Self {
        Self {
            base: Arc::clone(&self.base),
            rules: self.rules.clone(),
            header: self.header,
            items: self.items.clone(),
            txs: self.txs.clone(),
            // Matches the the reference implementation copy constructor, which leaves
            // `baseTxCount_` at its default zero value.
            base_tx_count: 0,
            open: self.open,
        }
    }
}

impl<B> OpenView<B>
where
    B: ReadView,
{
    pub fn new_open(base: Arc<B>, rules: Rules) -> Self {
        let mut header = base.header();
        header.validated = false;
        header.accepted = false;
        header.seq = header.seq.wrapping_add(1);
        header.parent_close_time = base.header().close_time;
        header.parent_hash = base.header().hash;

        Self {
            base,
            rules,
            header,
            items: RawStateTable::new(),
            txs: BTreeMap::new(),
            base_tx_count: 0,
            open: true,
        }
    }

    pub fn new_closed(base: Arc<B>) -> Self {
        Self {
            rules: base.rules(),
            header: base.header(),
            open: base.open(),
            base,
            items: RawStateTable::new(),
            txs: BTreeMap::new(),
            base_tx_count: 0,
        }
    }

    pub fn batch_from(base: Arc<Self>) -> OpenView<Self> {
        let base_tx_count = base.tx_count();
        OpenView {
            rules: base.rules(),
            header: base.header(),
            base,
            items: RawStateTable::new(),
            txs: BTreeMap::new(),
            base_tx_count,
            open: false,
        }
    }

    pub fn base(&self) -> &Arc<B> {
        &self.base
    }

    pub fn tx_count(&self) -> usize {
        self.base_tx_count + self.txs.len()
    }

    pub fn apply<T>(&self, to: &mut T) -> Result<(), ViewError>
    where
        T: TxsRawView,
    {
        self.items.apply(to)?;
        for (key, item) in &self.txs {
            to.raw_tx_insert(*key, Arc::clone(&item.txn), item.metadata.clone())?;
        }
        Ok(())
    }

    /// Apply only the state changes (not tx map entries) to a RawView.
    /// Used in catchup builds where tx map is handled separately.
    /// Includes XRP destruction accumulated via raw_destroy_xrp calls.
    pub fn apply_state_only<T>(&self, to: &mut T) -> Result<(), ViewError>
    where
        T: RawView,
    {
        self.items.apply(to)
    }
}

impl<B> ReadView for OpenView<B>
where
    B: ReadView,
{
    fn open(&self) -> bool {
        self.open
    }

    fn header(&self) -> LedgerHeader {
        self.header
    }

    fn fees(&self) -> Fees {
        self.base.fees()
    }

    fn rules(&self) -> Rules {
        self.rules.clone()
    }

    fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
        self.items.exists(self.base.as_ref(), keylet)
    }

    fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
        self.items.succ(self.base.as_ref(), key, last)
    }

    fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        self.items.read(self.base.as_ref(), keylet)
    }

    fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
        let mut current = Uint256::zero();
        let mut result = Vec::new();

        while let Some(next) = self.succ(current, None)? {
            if let Some(sle) = self.read(Keylet::new(LedgerEntryType::Any, next))? {
                result.push(sle);
            }
            current = next;
        }

        Ok(result)
    }

    fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
        Ok(self.txs.contains_key(&key))
    }

    fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
        if let Some(item) = self.txs.get(&key) {
            let tx = Arc::new(STTx::from_serial_iter(&mut protocol::SerialIter::new(
                item.txn.data(),
            )));
            let metadata = item.metadata.as_ref().map(|metadata| {
                Arc::new(STObject::from_serial_iter(
                    &mut protocol::SerialIter::new(metadata.data()),
                    protocol::get_field_by_symbol("sfMetadata"),
                    0,
                ))
            });
            return Ok(Some(ReadViewTx::new(tx, metadata)));
        }
        self.base.tx_read(key)
    }

    fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
        self.txs
            .keys()
            .copied()
            .map(|key| {
                self.tx_read(key)?
                    .ok_or_else(|| ViewError::Conversion("overlay tx disappeared".to_string()))
            })
            .collect()
    }

    fn balance_hook_iou(
        &self,
        account: protocol::AccountID,
        issuer: protocol::AccountID,
        amount: protocol::STAmount,
    ) -> protocol::STAmount {
        self.base.balance_hook_iou(account, issuer, amount)
    }

    fn balance_hook_mpt(
        &self,
        account: protocol::AccountID,
        issue: protocol::MPTIssue,
        amount: i64,
    ) -> protocol::STAmount {
        self.base.balance_hook_mpt(account, issue, amount)
    }

    fn balance_hook_self_issue_mpt(
        &self,
        issue: protocol::MPTIssue,
        amount: i64,
    ) -> protocol::STAmount {
        self.base.balance_hook_self_issue_mpt(issue, amount)
    }

    fn owner_count_hook(&self, account: protocol::AccountID, count: u32) -> u32 {
        self.base.owner_count_hook(account, count)
    }
}

impl<B> RawView for OpenView<B>
where
    B: ReadView,
{
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.items.erase(sle)
    }

    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.items.insert(sle)
    }

    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.items.replace(sle)
    }

    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        self.items.destroy_xrp(fee);
        Ok(())
    }
}

impl<B> TxsRawView for OpenView<B>
where
    B: ReadView,
{
    fn raw_tx_insert(
        &mut self,
        key: Uint256,
        txn: Arc<Serializer>,
        metadata: Option<Arc<Serializer>>,
    ) -> Result<(), ViewError> {
        if self.txs.insert(key, TxEntry { txn, metadata }).is_some() {
            return Err(ViewError::DuplicateTx(key));
        }
        Ok(())
    }
}
