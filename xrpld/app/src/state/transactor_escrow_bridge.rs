//! Escrow transactor apply bridge.

use basics::math::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ViewError, adjust_owner_count, dir_insert};
use protocol::{AccountID, Asset, STAmount, STLedgerEntry, Ter, XRPAmount, get_field_by_symbol};
use std::sync::Arc;
use tx::escrow::escrow_create::{EscrowCreateApplyFacts, EscrowCreateApplySink};

/// Check the source account's reserve after XRP is locked without relying on
/// signed subtraction. rippled's STAmount arithmetic permits an intermediate
/// negative value, whereas a Rust `i64` subtraction would panic in debug mode.
fn xrp_post_lock_reserve_sufficient(
    balance_drops: i64,
    escrow_drops: i64,
    reserve_drops: i64,
) -> bool {
    balance_drops
        .checked_sub(escrow_drops)
        .is_some_and(|remaining| remaining >= reserve_drops)
}

pub fn build_escrow_create_facts<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    dst_account: &AccountID,
    amount: &STAmount,
    finish_after: Option<u32>,
    cancel_after: Option<u32>,
) -> Result<EscrowCreateApplyFacts, ViewError> {
    let mut facts = EscrowCreateApplyFacts {
        amount_is_xrp: amount.native(),
        finish_after_expired: finish_after
            .is_some_and(|time| view.header().parent_close_time > time),
        cancel_after_expired: cancel_after
            .is_some_and(|time| view.header().parent_close_time > time),
        include_sequence_field: view
            .rules()
            .enabled(&protocol::feature_id("fixIncludeKeyletFields")),
        ..EscrowCreateApplyFacts::default()
    };

    if let Some(src_sle) =
        view.peek(protocol::account_keylet(Uint160::from_void(account.data())))?
    {
        facts.owner_exists = true;
        let reserve_drops = ledger::effective_account_reserve(view.fees(), &src_sle, 1, 0) as i64;
        let balance_drops = src_sle
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops();
        facts.reserve_sufficient = reserve_drops <= balance_drops;
        facts.xrp_balance_covers_amount = !amount.native()
            || xrp_post_lock_reserve_sufficient(balance_drops, amount.xrp().drops(), reserve_drops);
        facts.should_set_transfer_rate = !amount.native()
            && view.rules().enabled(&protocol::feature_token_escrow())
            && match amount.asset() {
                Asset::Issue(issue) => {
                    ledger::ripple_state_helpers::try_transfer_rate(view, &issue.issuer())?
                        != protocol::PARITY_RATE.value
                }
                Asset::MPTIssue(issue) => {
                    ledger::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())?.value
                        != protocol::PARITY_RATE.value
                }
            };
    }

    let destination = view.read(protocol::account_keylet(Uint160::from_void(
        dst_account.data(),
    )))?;
    facts.destination_exists = destination.is_some();
    facts.destination_requires_tag = destination
        .as_ref()
        .is_some_and(|sle| sle.is_flag(protocol::lsfRequireDestTag));
    facts.destination_is_sender = account == dst_account;
    facts.issuer_owner_dir_required = match amount.asset() {
        Asset::Issue(issue) if !issue.native() => {
            issue.issuer() != *account && issue.issuer() != *dst_account
        }
        Asset::MPTIssue(_) => false,
        _ => false,
    };
    Ok(facts)
}

pub struct ViewBackedEscrowCreateSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub dst_account: AccountID,
    pub amount: STAmount,
    pub escrow_key: Uint256,
    pub escrow_seq: u32,
    pub finish_after: Option<u32>,
    pub cancel_after: Option<u32>,
    pub condition: Option<Vec<u8>>,
    pub source_tag: Option<u32>,
    pub destination_tag: Option<u32>,
    pub reserve_sponsor: Option<Arc<STLedgerEntry>>,
    pub failure: Option<Ter>,
}

impl<'a, V: ApplyView> EscrowCreateApplySink for ViewBackedEscrowCreateSink<'a, V> {
    fn create_escrow_entry(&mut self) {
        let escrow_kl =
            protocol::escrow_keylet(Uint160::from_void(self.account.data()), self.escrow_seq);
        let mut sle = STLedgerEntry::new(escrow_kl);
        sle.set_account_id(get_field_by_symbol("sfAccount"), self.account);
        sle.set_account_id(get_field_by_symbol("sfDestination"), self.dst_account);
        sle.set_field_amount(get_field_by_symbol("sfAmount"), self.amount.clone());
        if let Some(finish_after) = self.finish_after {
            sle.set_field_u32(get_field_by_symbol("sfFinishAfter"), finish_after);
        }
        if let Some(cancel_after) = self.cancel_after {
            sle.set_field_u32(get_field_by_symbol("sfCancelAfter"), cancel_after);
        }
        if let Some(condition) = &self.condition {
            sle.set_field_vl(get_field_by_symbol("sfCondition"), condition);
        }
        if let Some(source_tag) = self.source_tag {
            sle.set_field_u32(get_field_by_symbol("sfSourceTag"), source_tag);
        }
        if let Some(destination_tag) = self.destination_tag {
            sle.set_field_u32(get_field_by_symbol("sfDestinationTag"), destination_tag);
        }
        if let Some(sponsor) = self.reserve_sponsor.as_ref() {
            sle.set_account_id(
                get_field_by_symbol("sfSponsor"),
                sponsor.get_account_id(get_field_by_symbol("sfAccount")),
            );
        }
        if self.view.insert(Arc::new(sle)).is_err() {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn set_sequence_field(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::escrow_keylet(
            Uint160::from_void(self.account.data()),
            self.escrow_seq,
        )) {
            let mut obj = sle.clone_as_object();
            obj.set_field_u32(get_field_by_symbol("sfSequence"), self.escrow_seq);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn set_transfer_rate(&mut self) {
        let rate = match self.amount.asset() {
            Asset::Issue(issue) => {
                match ledger::ripple_state_helpers::try_transfer_rate(self.view, &issue.issuer()) {
                    Ok(rate) => rate,
                    Err(_) => {
                        self.failure = Some(Ter::TEF_BAD_LEDGER);
                        return;
                    }
                }
            }
            Asset::MPTIssue(issue) => {
                match ledger::mptoken_helpers::transfer_rate_mpt(self.view, issue.mpt_id()) {
                    Ok(rate) => rate.value,
                    Err(_) => {
                        self.failure = Some(Ter::TEF_BAD_LEDGER);
                        return;
                    }
                }
            }
        };
        if let Ok(Some(sle)) = self.view.peek(protocol::escrow_keylet(
            Uint160::from_void(self.account.data()),
            self.escrow_seq,
        )) {
            let mut object = sle.clone_as_object();
            object.set_field_u32(get_field_by_symbol("sfTransferRate"), rate);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(object, *sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn insert_sender_owner_dir(&mut self) -> Option<u64> {
        let escrow_kl =
            protocol::escrow_keylet(Uint160::from_void(self.account.data()), self.escrow_seq);
        match dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            escrow_kl.key,
            &ledger::describe_owner_dir(self.account),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }
    fn set_sender_owner_node(&mut self, page: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::escrow_keylet(
            Uint160::from_void(self.account.data()),
            self.escrow_seq,
        )) {
            // Simplified
            let mut obj = sle.clone_as_object();
            obj.set_field_u64(get_field_by_symbol("sfOwnerNode"), page);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn insert_destination_owner_dir(&mut self) -> Option<u64> {
        let escrow_kl =
            protocol::escrow_keylet(Uint160::from_void(self.account.data()), self.escrow_seq);
        match dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.dst_account.data())),
            escrow_kl.key,
            &ledger::describe_owner_dir(self.dst_account),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }
    fn set_destination_owner_node(&mut self, page: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::escrow_keylet(
            Uint160::from_void(self.account.data()),
            self.escrow_seq,
        )) {
            // Simplified
            let mut obj = sle.clone_as_object();
            obj.set_field_u64(get_field_by_symbol("sfDestinationNode"), page);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn insert_issuer_owner_dir(&mut self) -> Option<u64> {
        let Asset::Issue(issue) = self.amount.asset() else {
            return None;
        };
        let escrow_kl =
            protocol::escrow_keylet(Uint160::from_void(self.account.data()), self.escrow_seq);
        match dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(issue.issuer().data())),
            escrow_kl.key,
            &ledger::describe_owner_dir(issue.issuer()),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }
    fn set_issuer_owner_node(&mut self, page: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::escrow_keylet(
            Uint160::from_void(self.account.data()),
            self.escrow_seq,
        )) {
            let mut obj = sle.clone_as_object();
            obj.set_field_u64(get_field_by_symbol("sfIssuerNode"), page);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn deduct_xrp_owner_balance(&mut self) {
        if let Ok(Some(src_sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let balance = src_sle.get_field_amount(get_field_by_symbol("sfBalance"));
            let Some(new_balance) = balance.xrp().drops().checked_sub(self.amount.xrp().drops())
            else {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return;
            };
            let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(new_balance));
            let mut obj = src_sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
            if self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())))
                .is_err()
            {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn lock_non_xrp_amount(&mut self) -> Ter {
        match self.amount.asset() {
            Asset::Issue(issue) => ledger::ripple_state_helpers::account_send(
                self.view,
                &self.account,
                &issue.issuer(),
                &self.amount,
            ),
            Asset::MPTIssue(_) => {
                ledger::mptoken_helpers::lock_escrow_mpt(self.view, &self.account, &self.amount)
                    .unwrap_or(Ter::TEF_BAD_LEDGER)
            }
        }
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(src_sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let result = if delta == 1 {
                ledger::increase_owner_count_for_object(
                    self.view,
                    &src_sle,
                    self.reserve_sponsor.as_ref(),
                )
            } else {
                adjust_owner_count(self.view, &src_sle, delta)
            };
            if result.is_err() {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
    fn update_owner(&mut self) {
        if let Ok(Some(src_sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            if self.view.update(src_sle).is_err() {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        } else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::xrp_post_lock_reserve_sufficient;

    #[test]
    fn xrp_post_lock_reserve_rejects_an_escrow_larger_than_the_source_balance() {
        assert!(!xrp_post_lock_reserve_sufficient(10, 11, 0));
    }

    #[test]
    fn xrp_post_lock_reserve_accepts_an_exact_reserve_remainder() {
        assert!(xrp_post_lock_reserve_sufficient(100, 60, 40));
    }
}
