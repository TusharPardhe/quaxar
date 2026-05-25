//! Rust port of `xrpl::PaymentSandbox` from `xrpl/ledger/PaymentSandbox.h`.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, ApplyFlags, Currency, Keylet, MPTID, MPTIssue, Rules, STAmount, STLedgerEntry,
    XRPAmount, sf_generic,
};

use crate::apply_state_table::ApplyStateTable;
use crate::raw_view::RawView;
use crate::read_view::{ReadView, ReadViewTx, ViewError};
use crate::{ApplyView, Fees, LedgerHeader};

#[derive(Debug, Default, Clone, PartialEq)]
struct ValueIOU {
    low_acct_credits: STAmount,
    high_acct_credits: STAmount,
    low_acct_orig_balance: STAmount,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct HolderValueMPT {
    debit: u64,
    orig_balance: u64,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct IssuerValueMPT {
    holders: BTreeMap<AccountID, HolderValueMPT>,
    credit: u64,
    orig_balance: i64,
    self_debit: u64,
}

#[derive(Debug, Default)]
struct DeferredCredits {
    credits_iou: BTreeMap<(AccountID, AccountID, Currency), ValueIOU>,
    credits_mpt: BTreeMap<MPTID, IssuerValueMPT>,
    owner_counts: BTreeMap<AccountID, u32>,
}

#[allow(dead_code)]
impl DeferredCredits {
    fn make_key_iou(
        a1: AccountID,
        a2: AccountID,
        currency: Currency,
    ) -> (AccountID, AccountID, Currency) {
        if a1 < a2 {
            (a1, a2, currency)
        } else {
            (a2, a1, currency)
        }
    }

    #[allow(dead_code)]
    fn credit_iou(
        &mut self,
        sender: AccountID,
        receiver: AccountID,
        amount: STAmount,
        pre_credit_sender_balance: STAmount,
    ) {
        let key = Self::make_key_iou(sender, receiver, amount.issue().currency);
        let entry = self.credits_iou.entry(key).or_insert_with(|| ValueIOU {
            low_acct_credits: STAmount::new_with_asset(sf_generic(), amount.issue(), 0, 0, false),
            high_acct_credits: STAmount::new_with_asset(sf_generic(), amount.issue(), 0, 0, false),
            low_acct_orig_balance: pre_credit_sender_balance,
        });

        if sender < receiver {
            entry.high_acct_credits += amount;
        } else {
            entry.low_acct_credits += amount;
        }
    }

    #[allow(dead_code)]
    fn credit_mpt(
        &mut self,
        sender: AccountID,
        _receiver: AccountID,
        amount: STAmount,
        pre_credit_balance_holder: u64,
        pre_credit_balance_issuer: i64,
    ) {
        let mpt_id = amount.asset().get::<MPTIssue>().mpt_id();
        let entry = self
            .credits_mpt
            .entry(mpt_id)
            .or_insert_with(|| IssuerValueMPT {
                holders: BTreeMap::new(),
                credit: 0,
                orig_balance: pre_credit_balance_issuer,
                self_debit: 0,
            });

        // credit to holder
        entry.credit += amount.mpt().value() as u64;

        // debit to issuer
        let holder_entry = entry
            .holders
            .entry(sender)
            .or_insert_with(|| HolderValueMPT {
                debit: 0,
                orig_balance: pre_credit_balance_holder,
            });
        holder_entry.debit += amount.mpt().value() as u64;
    }

    #[allow(dead_code)]
    fn issuer_self_debit_mpt(&mut self, issue: MPTIssue, amount: u64, orig_balance: i64) {
        let entry = self
            .credits_mpt
            .entry(issue.mpt_id())
            .or_insert_with(|| IssuerValueMPT {
                holders: BTreeMap::new(),
                credit: 0,
                orig_balance,
                self_debit: 0,
            });
        entry.self_debit += amount;
    }

    #[allow(dead_code)]
    fn owner_count(&mut self, id: AccountID, _cur: u32, next: u32) {
        let entry = self.owner_counts.entry(id).or_insert(next);
        if next > *entry {
            *entry = next;
        }
    }

    fn get_owner_count(&self, id: AccountID) -> Option<u32> {
        self.owner_counts.get(&id).copied()
    }

    fn apply(&self, to: &mut DeferredCredits) {
        for (key, value) in &self.credits_iou {
            let to_entry = to.credits_iou.entry(*key).or_insert_with(|| value.clone());
            if to_entry != value {
                to_entry.low_acct_credits += value.low_acct_credits.clone();
                to_entry.high_acct_credits += value.high_acct_credits.clone();
            }
        }
        for (mpt_id, value) in &self.credits_mpt {
            let to_entry = to
                .credits_mpt
                .entry(*mpt_id)
                .or_insert_with(|| value.clone());
            if to_entry != value {
                to_entry.credit += value.credit;
                to_entry.self_debit += value.self_debit;
                for (holder, holder_val) in &value.holders {
                    let to_holder = to_entry
                        .holders
                        .entry(*holder)
                        .or_insert_with(|| holder_val.clone());
                    if to_holder != holder_val {
                        to_holder.debit += holder_val.debit;
                    }
                }
            }
        }
        for (id, count) in &self.owner_counts {
            let to_count = to.owner_counts.entry(*id).or_insert(*count);
            if *count > *to_count {
                *to_count = *count;
            }
        }
    }
}

#[derive(Debug)]
pub struct PaymentSandbox<B> {
    base: Arc<B>,
    table: ApplyStateTable,
    tab: DeferredCredits,
    flags: ApplyFlags,
}

impl<B> PaymentSandbox<B>
where
    B: ReadView,
{
    pub fn new(base: Arc<B>, flags: ApplyFlags) -> Self {
        Self {
            base,
            table: ApplyStateTable::new(),
            tab: DeferredCredits::default(),
            flags,
        }
    }

    pub fn apply(&self, to: &mut dyn RawView) -> Result<(), ViewError> {
        self.table.apply(to)
    }

    pub fn apply_to_sandbox(&self, to: &mut PaymentSandbox<B>) -> Result<(), ViewError> {
        self.table.apply(to)?;
        self.tab.apply(&mut to.tab);
        Ok(())
    }
}

impl<B> ReadView for PaymentSandbox<B>
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
        mut amount: STAmount,
    ) -> STAmount {
        let key = DeferredCredits::make_key_iou(account, issuer, amount.issue().currency);
        if let Some(entry) = self.tab.credits_iou.get(&key) {
            if account < issuer {
                amount -= entry.low_acct_credits.clone();
            } else {
                amount -= entry.high_acct_credits.clone();
            }
        }
        self.base.balance_hook_iou(account, issuer, amount)
    }

    fn balance_hook_mpt(&self, account: AccountID, issue: MPTIssue, mut amount: i64) -> STAmount {
        if let Some(entry) = self.tab.credits_mpt.get(&issue.mpt_id())
            && let Some(holder_entry) = entry.holders.get(&account)
        {
            amount -= holder_entry.debit as i64;
        }
        self.base.balance_hook_mpt(account, issue, amount)
    }

    fn balance_hook_self_issue_mpt(&self, issue: MPTIssue, mut amount: i64) -> STAmount {
        if let Some(entry) = self.tab.credits_mpt.get(&issue.mpt_id()) {
            amount -= entry.self_debit as i64;
        }
        self.base.balance_hook_self_issue_mpt(issue, amount)
    }

    fn owner_count_hook(&self, account: AccountID, count: u32) -> u32 {
        let count = self.tab.get_owner_count(account).unwrap_or(count);
        self.base.owner_count_hook(account, count)
    }
}

impl<B> RawView for PaymentSandbox<B>
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

impl<B> ApplyView for PaymentSandbox<B>
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
