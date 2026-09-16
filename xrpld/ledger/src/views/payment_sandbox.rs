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
pub(crate) struct DeferredCredits {
    credits_iou: BTreeMap<(AccountID, AccountID, Currency), ValueIOU>,
    credits_mpt: BTreeMap<MPTID, IssuerValueMPT>,
    owner_counts: BTreeMap<AccountID, crate::OwnerCounts>,
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
    pub(crate) fn credit_iou(
        &mut self,
        sender: AccountID,
        receiver: AccountID,
        amount: STAmount,
        pre_credit_sender_balance: STAmount,
    ) {
        let sender_is_low = sender < receiver;
        let key = Self::make_key_iou(sender, receiver, amount.issue().currency);
        let entry = self.credits_iou.entry(key).or_insert_with(|| ValueIOU {
            low_acct_credits: STAmount::new_with_asset(sf_generic(), amount.issue(), 0, 0, false),
            high_acct_credits: STAmount::new_with_asset(sf_generic(), amount.issue(), 0, 0, false),
            low_acct_orig_balance: if sender_is_low {
                pre_credit_sender_balance
            } else {
                let mut balance = pre_credit_sender_balance;
                balance.negate();
                balance
            },
        });

        if sender_is_low {
            entry.low_acct_credits += amount;
        } else {
            entry.high_acct_credits += amount;
        }
    }

    #[allow(dead_code)]
    pub(crate) fn credit_mpt(
        &mut self,
        sender: AccountID,
        receiver: AccountID,
        amount: STAmount,
        pre_credit_balance_holder: u64,
        pre_credit_balance_issuer: i64,
    ) {
        let asset = amount.asset();
        let issue = asset.get::<MPTIssue>();
        let value = amount.mpt().value() as u64;
        let sender_is_issuer = sender == issue.issuer();
        let entry = self
            .credits_mpt
            .entry(issue.mpt_id())
            .or_insert_with(|| IssuerValueMPT {
                holders: BTreeMap::new(),
                credit: 0,
                orig_balance: pre_credit_balance_issuer,
                self_debit: 0,
            });

        if sender_is_issuer {
            entry.credit = entry.credit.wrapping_add(value);
            entry
                .holders
                .entry(receiver)
                .or_insert_with(|| HolderValueMPT {
                    debit: 0,
                    orig_balance: pre_credit_balance_holder,
                });
        } else {
            let holder = entry
                .holders
                .entry(sender)
                .or_insert_with(|| HolderValueMPT {
                    debit: 0,
                    orig_balance: pre_credit_balance_holder,
                });
            holder.debit = holder.debit.wrapping_add(value);
        }
    }

    pub(crate) fn issuer_self_debit_mpt(
        &mut self,
        issue: MPTIssue,
        amount: u64,
        orig_balance: i64,
    ) {
        let entry = self
            .credits_mpt
            .entry(issue.mpt_id())
            .or_insert_with(|| IssuerValueMPT {
                holders: BTreeMap::new(),
                credit: 0,
                orig_balance,
                self_debit: 0,
            });
        entry.self_debit = entry.self_debit.wrapping_add(amount);
    }

    #[allow(dead_code)]
    pub(crate) fn owner_count(
        &mut self,
        id: AccountID,
        cur: crate::OwnerCounts,
        next: crate::OwnerCounts,
    ) {
        let reached = cur.max(next);
        let entry = self.owner_counts.entry(id).or_insert(reached);
        *entry = (*entry).max(reached);
    }

    pub(crate) fn get_owner_count(&self, id: AccountID) -> Option<crate::OwnerCounts> {
        self.owner_counts.get(&id).copied()
    }

    pub(crate) fn apply(&self, to: &mut DeferredCredits) {
        for (key, value) in &self.credits_iou {
            let to_entry = to.credits_iou.entry(*key).or_insert_with(|| value.clone());
            if to_entry != value {
                to_entry.low_acct_credits += value.low_acct_credits.clone();
                to_entry.high_acct_credits += value.high_acct_credits.clone();
            }
        }
        for (mpt_id, value) in &self.credits_mpt {
            if let Some(to_entry) = to.credits_mpt.get_mut(mpt_id) {
                to_entry.credit = to_entry.credit.wrapping_add(value.credit);
                to_entry.self_debit = to_entry.self_debit.wrapping_add(value.self_debit);
                for (holder, holder_val) in &value.holders {
                    if let Some(to_holder) = to_entry.holders.get_mut(holder) {
                        to_holder.debit = to_holder.debit.wrapping_add(holder_val.debit);
                    } else {
                        to_entry.holders.insert(*holder, holder_val.clone());
                    }
                }
                // The destination already owns the earliest original balances.
            } else {
                to.credits_mpt.insert(*mpt_id, value.clone());
            }
        }
        for (id, count) in &self.owner_counts {
            let to_count = to.owner_counts.entry(*id).or_insert(*count);
            if *count > *to_count {
                *to_count = *count;
            }
        }
    }

    pub(crate) fn balance_iou(
        &self,
        account: AccountID,
        issuer: AccountID,
        mut amount: STAmount,
    ) -> STAmount {
        let key = Self::make_key_iou(account, issuer, amount.issue().currency);
        if let Some(entry) = self.credits_iou.get(&key) {
            let (debits, original_balance) = if account < issuer {
                (
                    entry.low_acct_credits.clone(),
                    entry.low_acct_orig_balance.clone(),
                )
            } else {
                let mut original_balance = entry.low_acct_orig_balance.clone();
                original_balance.negate();
                (entry.high_acct_credits.clone(), original_balance)
            };
            amount = amount
                .min(original_balance.clone() - debits)
                .min(original_balance);
            if issuer == protocol::xrp_account() && amount.signum() < 0 {
                amount = amount.zeroed();
            }
        }
        amount
    }

    pub(crate) fn balance_mpt(&self, account: AccountID, issue: MPTIssue, amount: i64) -> i64 {
        let Some(entry) = self.credits_mpt.get(&issue.mpt_id()) else {
            return amount.max(0);
        };
        let (delta, original) = if account == issue.issuer() {
            (entry.credit as i64, entry.orig_balance)
        } else if let Some(holder) = entry.holders.get(&account) {
            (holder.debit as i64, holder.orig_balance as i64)
        } else {
            return amount.max(0);
        };
        amount
            .min(original.wrapping_sub(delta))
            .min(original)
            .max(0)
    }

    pub(crate) fn balance_self_issue_mpt(&self, issue: MPTIssue, amount: i64) -> i64 {
        let Some(entry) = self.credits_mpt.get(&issue.mpt_id()) else {
            return amount.max(0);
        };
        amount
            .min(entry.orig_balance.wrapping_sub(entry.self_debit as i64))
            .max(0)
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
        amount = self.tab.balance_iou(account, issuer, amount);
        self.base.balance_hook_iou(account, issuer, amount)
    }

    fn balance_hook_mpt(&self, account: AccountID, issue: MPTIssue, mut amount: i64) -> STAmount {
        amount = self.tab.balance_mpt(account, issue, amount);
        self.base.balance_hook_mpt(account, issue, amount)
    }

    fn balance_hook_self_issue_mpt(&self, issue: MPTIssue, amount: i64) -> STAmount {
        let amount = self.tab.balance_self_issue_mpt(issue, amount);
        self.base.balance_hook_self_issue_mpt(issue, amount)
    }

    fn owner_count_hook(
        &self,
        account: AccountID,
        count: crate::OwnerCounts,
    ) -> crate::OwnerCounts {
        let count = self
            .tab
            .get_owner_count(account)
            .unwrap_or(count)
            .max(count);
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

    fn credit_hook_iou(
        &mut self,
        from: AccountID,
        to: AccountID,
        amount: STAmount,
        pre_credit_balance: STAmount,
    ) {
        self.tab.credit_iou(from, to, amount, pre_credit_balance);
    }

    fn credit_hook_mpt(
        &mut self,
        from: AccountID,
        to: AccountID,
        amount: STAmount,
        pre_credit_balance_holder: u64,
        pre_credit_balance_issuer: i64,
    ) {
        self.tab.credit_mpt(
            from,
            to,
            amount,
            pre_credit_balance_holder,
            pre_credit_balance_issuer,
        );
    }

    fn issuer_self_debit_hook_mpt(&mut self, issue: MPTIssue, amount: u64, orig_balance: i64) {
        self.tab.issuer_self_debit_mpt(issue, amount, orig_balance);
    }

    fn adjust_owner_count_hook(
        &mut self,
        account: AccountID,
        cur: crate::OwnerCounts,
        next: crate::OwnerCounts,
    ) {
        self.tab.owner_count(account, cur, next);
    }
}

#[cfg(test)]
mod tests {
    use super::DeferredCredits;
    use crate::OwnerCounts;
    use protocol::{AccountID, MPTAmount, MPTIssue, STAmount, get_field_by_symbol};

    #[test]
    fn deferred_mpt_credits_and_self_debits_cannot_be_reused() {
        let issuer = AccountID::from_array([0x31; 20]);
        let holder = AccountID::from_array([0x32; 20]);
        let issue = MPTIssue::new(protocol::make_mpt_id(1, issuer));
        let amount = STAmount::from_mpt_amount(
            get_field_by_symbol("sfAmount"),
            MPTAmount::from_value(1),
            issue,
        );
        let mut credits = DeferredCredits::default();

        credits.credit_mpt(issuer, holder, amount, 0, 0);
        assert_eq!(credits.balance_mpt(holder, issue, 1), 0);
        assert_eq!(credits.balance_mpt(issuer, issue, -1), 0);

        let self_issue = MPTIssue::new(protocol::make_mpt_id(2, issuer));
        credits.issuer_self_debit_mpt(self_issue, 3, 10);
        assert_eq!(credits.balance_self_issue_mpt(self_issue, 10), 7);
    }

    #[test]
    fn applying_equal_shaped_mpt_credit_tables_accumulates_both() {
        let issuer = AccountID::from_array([0x41; 20]);
        let holder = AccountID::from_array([0x42; 20]);
        let issue = MPTIssue::new(protocol::make_mpt_id(1, issuer));
        let amount = STAmount::from_mpt_amount(
            get_field_by_symbol("sfAmount"),
            MPTAmount::from_value(1),
            issue,
        );
        let mut first = DeferredCredits::default();
        let mut second = DeferredCredits::default();
        first.credit_mpt(issuer, holder, amount.clone(), 0, 10);
        second.credit_mpt(issuer, holder, amount, 0, 10);

        second.apply(&mut first);
        assert_eq!(first.balance_mpt(issuer, issue, 10), 8);
    }

    #[test]
    fn deferred_owner_counts_compare_effective_sponsor_reserve() {
        let account =
            AccountID::from_hex("0707070707070707070707070707070707070707").expect("valid account");
        let mut credits = DeferredCredits::default();

        // Adding a sponsored object raises both owner and sponsored, so the
        // effective reserve count remains one. A later unsponsored object
        // raises the effective count to two and must become the retained max.
        credits.owner_count(
            account,
            OwnerCounts {
                owner: 1,
                sponsored: 0,
                sponsoring: 0,
            },
            OwnerCounts {
                owner: 2,
                sponsored: 1,
                sponsoring: 0,
            },
        );
        assert_eq!(credits.get_owner_count(account).unwrap().count(), 1);

        credits.owner_count(
            account,
            OwnerCounts {
                owner: 2,
                sponsored: 1,
                sponsoring: 0,
            },
            OwnerCounts {
                owner: 3,
                sponsored: 1,
                sponsoring: 0,
            },
        );
        assert_eq!(credits.get_owner_count(account).unwrap().count(), 2);
    }
}
