//! Rust port of `xrpl::Sandbox` from `xrpl/ledger/Sandbox.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, ApplyFlags, Keylet, MPTIssue, Rules, STAmount, STLedgerEntry, XRPAmount,
};

use crate::apply_state_table::ApplyStateTable;
use crate::raw_view::RawView;
use crate::read_view::{ReadView, ReadViewTx, ViewError};
use crate::{ApplyView, Fees, LedgerHeader};

#[derive(Debug)]
pub struct Sandbox<B> {
    base: Arc<B>,
    table: ApplyStateTable,
    flags: ApplyFlags,
}

impl<B> Sandbox<B>
where
    B: ReadView,
{
    pub fn new(base: Arc<B>, flags: ApplyFlags) -> Self {
        Self {
            base,
            table: ApplyStateTable::new(),
            flags,
        }
    }

    pub fn apply(&self, to: &mut dyn RawView) -> Result<(), ViewError> {
        self.table.apply(to)
    }

    pub fn apply_with_tx_thread(
        &self,
        to: &mut dyn RawView,
        tx_id: Uint256,
        ledger_seq: u32,
        rules: &Rules,
    ) -> Result<(), ViewError> {
        self.table
            .apply_with_tx_thread(to, tx_id, ledger_seq, rules)
    }

    /// Debug: return a summary of all state modifications in this sandbox.
    pub fn modification_summary(&self) -> String {
        self.table.modification_summary()
    }

    pub fn modification_debug_lines(&self) -> Vec<String> {
        self.table.modification_debug_lines()
    }
}

impl<B> ReadView for Sandbox<B>
where
    B: ReadView,
{
    fn open(&self) -> bool {
        self.base.open()
    }

    fn header(&self) -> LedgerHeader {
        self.base.header()
    }

    fn fees(&self) -> Fees {
        self.base.fees()
    }

    fn rules(&self) -> Rules {
        self.base.rules()
    }

    fn exists(&self, k: Keylet) -> Result<bool, ViewError> {
        self.table.exists(self.base.as_ref(), k)
    }

    fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
        self.table.succ(self.base.as_ref(), key, last)
    }

    fn read(&self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        self.table.read(self.base.as_ref(), k)
    }

    fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
        self.base.sles()
    }

    fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
        self.base.tx_exists(key)
    }

    fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
        self.base.tx_read(key)
    }

    fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
        self.base.txs()
    }

    fn balance_hook_iou(
        &self,
        account: AccountID,
        issuer: AccountID,
        amount: STAmount,
    ) -> STAmount {
        self.base.balance_hook_iou(account, issuer, amount)
    }

    fn balance_hook_mpt(&self, account: AccountID, issue: MPTIssue, amount: i64) -> STAmount {
        self.base.balance_hook_mpt(account, issue, amount)
    }

    fn balance_hook_self_issue_mpt(&self, issue: MPTIssue, amount: i64) -> STAmount {
        self.base.balance_hook_self_issue_mpt(issue, amount)
    }

    fn owner_count_hook(&self, account: AccountID, count: u32) -> u32 {
        self.base.owner_count_hook(account, count)
    }
}

impl<B> RawView for Sandbox<B>
where
    B: ReadView,
{
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.table.erase(self.base.as_ref(), sle)
    }

    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.table.insert(self.base.as_ref(), sle)
    }

    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.table.replace(self.base.as_ref(), sle)
    }

    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        self.table.destroy_xrp(fee);
        Ok(())
    }
}

impl<B> ApplyView for Sandbox<B>
where
    B: ReadView,
{
    fn flags(&self) -> ApplyFlags {
        self.flags
    }

    fn peek(&mut self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        self.table.peek(self.base.as_ref(), k)
    }

    fn insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.raw_insert(sle)
    }

    fn update(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.table.update(self.base.as_ref(), sle)
    }

    fn erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.raw_erase(sle)
    }

    fn destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        self.raw_destroy_xrp(fee)
    }
}
