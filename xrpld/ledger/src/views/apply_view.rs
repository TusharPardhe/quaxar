//! Rust port of `xrpl::ApplyView` from `xrpl/ledger/ApplyView.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, ApplyFlags, Keylet, MPTIssue, Rules, STAmount, STLedgerEntry, XRPAmount,
};

use crate::Fees;
use crate::apply_state_table::ApplyStateTable;
use crate::raw_view::RawView;
use crate::read_view::{ReadView, ViewError};

///
/// Provides the `peek`/`insert`/`update`/`erase` SLE checkout pattern,
/// directory helpers (`dirAppend`/`dirInsert`/`dirRemove`), and
/// owner-count adjustment hooks.
pub trait ApplyView: ReadView + RawView {
    fn flags(&self) -> ApplyFlags;

    /// Checkout a mutable SLE. Returns `None` if the key doesn't exist.
    fn peek(&mut self, k: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError>;

    /// Insert a new SLE into the view.
    fn insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;

    /// Mark a previously peeked SLE as updated.
    fn update(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;

    /// Erase a previously peeked SLE.
    fn erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;

    /// Destroy XRP (fee collection).
    fn destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError>;

    /// Hook called when IOU credit occurs (overridden by PaymentSandbox).
    fn credit_hook_iou(
        &mut self,
        _from: AccountID,
        _to: AccountID,
        _amount: STAmount,
        _pre_credit_balance: STAmount,
    ) {
    }

    /// Hook called when MPT credit occurs.
    fn credit_hook_mpt(
        &mut self,
        _from: AccountID,
        _to: AccountID,
        _amount: STAmount,
        _pre_credit_balance_holder: u64,
        _pre_credit_balance_issuer: i64,
    ) {
    }

    /// Hook called when owner count changes (overridden by PaymentSandbox).
    fn adjust_owner_count_hook(&mut self, _account: AccountID, _cur: u32, _next: u32) {}
}

// ---------------------------------------------------------------------------
// ApplyViewImpl — concrete implementation backed by ApplyStateTable
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ApplyViewImpl<B> {
    base: Arc<B>,
    table: ApplyStateTable,
    flags: ApplyFlags,
}

impl<B> ApplyViewImpl<B>
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

    pub fn table(&self) -> &ApplyStateTable {
        &self.table
    }
}

impl<B> ReadView for ApplyViewImpl<B>
where
    B: ReadView,
{
    fn open(&self) -> bool {
        self.base.open()
    }

    fn header(&self) -> crate::LedgerHeader {
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

    fn tx_read(&self, key: Uint256) -> Result<Option<crate::read_view::ReadViewTx>, ViewError> {
        self.base.tx_read(key)
    }

    fn txs(&self) -> Result<Vec<crate::read_view::ReadViewTx>, ViewError> {
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

impl<B> RawView for ApplyViewImpl<B>
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

impl<B> ApplyView for ApplyViewImpl<B>
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

// ---------------------------------------------------------------------------
// Free-standing ledger helpers matching reference AccountRootHelpers.h
// ---------------------------------------------------------------------------

fn confine_owner_count(current: u32, adjustment: i32) -> u32 {
    let result = current as i64 + adjustment as i64;
    if result < 0 {
        0
    } else if result > u32::MAX as i64 {
        u32::MAX
    } else {
        result as u32
    }
}

pub fn adjust_owner_count(
    view: &mut dyn ApplyView,
    sle: &Arc<STLedgerEntry>,
    amount: i32,
) -> Result<(), ViewError> {
    let current = sle.get_field_u32(protocol::get_field_by_symbol("sfOwnerCount"));
    let account = sle.get_account_id(protocol::get_field_by_symbol("sfAccount"));
    let adjusted = confine_owner_count(current, amount);
    view.adjust_owner_count_hook(account, current, adjusted);
    let mut updated = sle.clone_as_object();
    updated.set_field_u32(protocol::get_field_by_symbol("sfOwnerCount"), adjusted);
    view.update(Arc::new(STLedgerEntry::from_stobject(updated, *sle.key())))
}

pub fn is_global_frozen(view: &dyn ReadView, issuer: &AccountID) -> Result<bool, ViewError> {
    if issuer.is_zero() {
        return Ok(false);
    }
    let account_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(issuer.data()));
    let Some(sle) = view.read(account_keylet)? else {
        return Ok(false);
    };
    let flags = sle.get_field_u32(protocol::get_field_by_symbol("sfFlags"));
    // lsfGlobalFreeze = 0x00400000
    Ok(flags & 0x0040_0000 != 0)
}

pub fn xrp_liquid(
    view: &dyn ReadView,
    id: &AccountID,
    owner_count_adj: i32,
) -> Result<XRPAmount, ViewError> {
    let account_keylet = protocol::account_keylet(basics::base_uint::Uint160::from_void(id.data()));
    let Some(sle) = view.read(account_keylet)? else {
        return Ok(XRPAmount::new());
    };
    let balance = sle.get_field_amount(protocol::get_field_by_symbol("sfBalance"));
    let owner_count = sle.get_field_u32(protocol::get_field_by_symbol("sfOwnerCount"));
    let adjusted_count = confine_owner_count(owner_count, owner_count_adj);
    let reserve = view.fees().account_reserve(adjusted_count as usize) as i64;
    let liquid = balance.xrp().drops() - reserve;
    Ok(if liquid < 0 {
        XRPAmount::new()
    } else {
        XRPAmount::from_drops(liquid)
    })
}
