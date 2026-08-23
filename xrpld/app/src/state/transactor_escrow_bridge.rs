//! Escrow transactor apply bridge.

use basics::math::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ViewError, adjust_owner_count, dir_insert, dir_remove};
use protocol::{AccountID, Asset, STAmount, STLedgerEntry, Ter, XRPAmount, get_field_by_symbol};
use std::sync::Arc;
use tx::escrow::escrow_cancel::EscrowCancelApplySink;
use tx::escrow::escrow_create::{EscrowCreateApplyFacts, EscrowCreateApplySink};
use tx::escrow::escrow_finish::EscrowFinishApplySink;

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
        let owner_count = src_sle.get_field_u32(get_field_by_symbol("sfOwnerCount"));
        let reserve_drops = view.fees().account_reserve(owner_count as usize + 1) as i64;
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
                    ledger::ripple_state_helpers::transfer_rate(view, &issue.issuer())
                        != protocol::PARITY_RATE.value
                }
                Asset::MPTIssue(issue) => {
                    ledger::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())
                        .map(|rate| rate.value)
                        .unwrap_or(protocol::PARITY_RATE.value)
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
                ledger::ripple_state_helpers::transfer_rate(self.view, &issue.issuer())
            }
            Asset::MPTIssue(issue) => {
                ledger::mptoken_helpers::transfer_rate_mpt(self.view, issue.mpt_id())
                    .map(|rate| rate.value)
                    .unwrap_or(protocol::PARITY_RATE.value)
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
                    .unwrap_or(Ter::TEF_INTERNAL)
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

pub struct ViewBackedEscrowFinishSink<'a, V> {
    pub view: &'a mut V,
    pub owner: AccountID,
    pub destination: AccountID,
    pub amount: STAmount,
    pub escrow_key: Uint256,
}

impl<'a, V: ApplyView> EscrowFinishApplySink for ViewBackedEscrowFinishSink<'a, V> {
    fn transfer_escrow_amount(&mut self) -> Ter {
        if self.amount.native() {
            let dst_keylet = protocol::account_keylet(Uint160::from_void(self.destination.data()));
            if let Ok(Some(dst_sle)) = self.view.peek(dst_keylet) {
                let dst_balance = dst_sle.get_field_amount(get_field_by_symbol("sfBalance"));
                let new_dst_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                    dst_balance.xrp().drops() + self.amount.xrp().drops(),
                ));
                let mut dst_obj = dst_sle.clone_as_object();
                dst_obj.set_field_amount(get_field_by_symbol("sfBalance"), new_dst_balance);
                let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
                    dst_obj,
                    *dst_sle.key(),
                )));
            }
            Ter::TES_SUCCESS
        } else {
            ledger::ripple_state_helpers::account_send(
                self.view,
                &self.owner,
                &self.destination,
                &self.amount,
            )
        }
    }
    fn remove_escrow_entry(&mut self) {
        let keylet = protocol::escrow_keylet_from_key(self.escrow_key);
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let owner_node = sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
            let _ = dir_remove(
                self.view,
                &protocol::owner_dir_keylet(Uint160::from_void(self.owner.data())),
                owner_node,
                *sle.key(),
                false,
            );
            if sle.has_field(get_field_by_symbol("sfDestinationNode")) {
                let dst_node = sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
                let _ = dir_remove(
                    self.view,
                    &protocol::owner_dir_keylet(Uint160::from_void(self.destination.data())),
                    dst_node,
                    *sle.key(),
                    false,
                );
            }
            let _ = self.view.erase(sle);
        }
    }
    fn adjust_owner_count(&mut self, account: &protocol::AccountID, delta: i32) {
        let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
}

pub struct ViewBackedEscrowCancelSink<'a, V> {
    pub view: &'a mut V,
    pub owner: AccountID,
    pub destination: AccountID,
    pub amount: STAmount,
    pub escrow_key: Uint256,
}

impl<'a, V: ApplyView> EscrowCancelApplySink for ViewBackedEscrowCancelSink<'a, V> {
    fn escrow_exists(&mut self) -> bool {
        self.view
            .exists(protocol::escrow_keylet_from_key(self.escrow_key))
            .unwrap_or(false)
    }
    fn token_escrow_enabled(&mut self) -> bool {
        false
    }
    fn cancel_after_present(&mut self) -> bool {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            return sle.has_field(get_field_by_symbol("sfCancelAfter"));
        }
        false
    }
    fn cancel_after_passed(&mut self) -> bool {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            if sle.has_field(get_field_by_symbol("sfCancelAfter")) {
                let cancel_after = sle.get_field_u32(get_field_by_symbol("sfCancelAfter"));
                return self.view.header().parent_close_time > cancel_after;
            }
        }
        false
    }
    fn remove_owner_dir(&mut self) -> bool {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            let owner_node = sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
            return dir_remove(
                self.view,
                &protocol::owner_dir_keylet(Uint160::from_void(self.owner.data())),
                owner_node,
                *sle.key(),
                false,
            )
            .is_ok();
        }
        false
    }
    fn destination_node_present(&mut self) -> bool {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            return sle.has_field(get_field_by_symbol("sfDestinationNode"));
        }
        false
    }
    fn remove_destination_dir(&mut self) -> bool {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            if sle.has_field(get_field_by_symbol("sfDestinationNode")) {
                let dst_node = sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
                return dir_remove(
                    self.view,
                    &protocol::owner_dir_keylet(Uint160::from_void(self.destination.data())),
                    dst_node,
                    *sle.key(),
                    false,
                )
                .is_ok();
            }
        }
        false
    }
    fn amount_is_xrp(&mut self) -> bool {
        self.amount.native()
    }
    fn credit_owner_xrp(&mut self) {
        let keylet = protocol::account_keylet(Uint160::from_void(self.owner.data()));
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
            let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                balance.xrp().drops() + self.amount.xrp().drops(),
            ));
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn apply_token_unlock(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn issuer_node_present(&mut self) -> bool {
        false
    }
    fn remove_issuer_dir(&mut self) -> bool {
        true
    }
    fn owner_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(Uint160::from_void(
                self.owner.data(),
            )))
            .unwrap_or(false)
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.owner.data(),
        ))) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
    fn update_owner(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.owner.data(),
        ))) {
            let _ = self.view.update(sle);
        }
    }
    fn erase_escrow(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::escrow_keylet_from_key(self.escrow_key))
        {
            let _ = self.view.erase(sle);
        }
    }
}
