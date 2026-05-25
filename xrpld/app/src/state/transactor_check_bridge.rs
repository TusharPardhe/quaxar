//! Check and PayChan transactor apply bridge.

use basics::math::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, adjust_owner_count, dir_insert, dir_remove};
use protocol::{AccountID, STAmount, STLedgerEntry, Ter, XRPAmount, get_field_by_symbol};
use std::sync::Arc;
use tx::check::check_cancel::CheckCancelApplySink;
use tx::check::check_cash::{CheckCashApplySink, CheckCashIouFlowResult};
use tx::check::check_create::{CheckCreateApplySink, CheckCreateMutation};
use tx::check::payment_channel_claim::PaymentChannelClaimApplySink;
use tx::check::payment_channel_create::PaymentChannelCreateApplySink;
use tx::check::payment_channel_fund::PaymentChannelFundApplySink;
use tx::check::payment_channel_helpers::PaymentChannelCloseSink;

pub struct ViewBackedCheckCreateSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub dst_account: AccountID,
    pub amount: STAmount,
    pub check_key: Uint256,
}

impl<'a, V: ApplyView> CheckCreateApplySink for ViewBackedCheckCreateSink<'a, V> {
    fn source_account_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(Uint160::from_void(
                self.account.data(),
            )))
            .unwrap_or(false)
    }
    fn reserve_sufficient(&mut self) -> bool {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let owner_count = sle.get_field_u32(get_field_by_symbol("sfOwnerCount"));
            let reserve = self.view.fees().account_reserve(owner_count as usize + 1);
            let balance = sle
                .get_field_amount(get_field_by_symbol("sfBalance"))
                .xrp()
                .drops();
            return balance >= reserve as i64;
        }
        false
    }
    fn insert_destination_dir(&mut self) -> Option<u64> {
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.dst_account.data())),
            self.check_key,
            &|_| {},
        )
        .ok()
        .flatten()
    }
    fn insert_owner_dir(&mut self) -> Option<u64> {
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            self.check_key,
            &|_| {},
        )
        .ok()
        .flatten()
    }
    fn create_check(&mut self, mutation: CheckCreateMutation) {
        let mut sle = STLedgerEntry::new(protocol::check_keylet(
            Uint160::from_void(self.account.data()),
            mutation.sequence,
        ));
        sle.set_account_id(get_field_by_symbol("sfAccount"), self.account);
        sle.set_account_id(get_field_by_symbol("sfDestination"), self.dst_account);
        sle.set_field_amount(get_field_by_symbol("sfSendMax"), self.amount.clone());
        sle.set_field_u32(get_field_by_symbol("sfSequence"), mutation.sequence);
        if let Some(tag) = mutation.source_tag {
            sle.set_field_u32(get_field_by_symbol("sfSourceTag"), tag);
        }
        if let Some(tag) = mutation.destination_tag {
            sle.set_field_u32(get_field_by_symbol("sfDestinationTag"), tag);
        }
        if let Some(invoice_id) = mutation.invoice_id {
            sle.set_field_h256(
                get_field_by_symbol("sfInvoiceID"),
                Uint256::from(invoice_id),
            );
        }
        if let Some(expiration) = mutation.expiration {
            sle.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
        }
        sle.set_field_u64(get_field_by_symbol("sfOwnerNode"), mutation.owner_node);
        if let Some(dst_node) = mutation.destination_node {
            sle.set_field_u64(get_field_by_symbol("sfDestinationNode"), dst_node);
        }
        let _ = self.view.insert(Arc::new(sle));
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
}

pub struct ViewBackedCheckCancelSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub check_key: Uint256,
}

impl<'a, V: ApplyView> ViewBackedCheckCancelSink<'a, V> {
    pub fn remove_check_entry(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) {
            let _ = self.view.erase(sle);
        }
    }
}

impl<'a, V: ApplyView> CheckCancelApplySink for ViewBackedCheckCancelSink<'a, V> {
    fn check_exists(&mut self) -> bool {
        self.view
            .exists(protocol::unchecked_keylet(self.check_key))
            .unwrap_or(false)
    }
    fn check_source_matches_destination(&mut self) -> bool {
        true
    }
    fn remove_destination_dir(&mut self) -> bool {
        true
    }
    fn remove_owner_dir(&mut self) -> bool {
        true
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
    fn erase_check(&mut self) {
        self.remove_check_entry();
    }
}

pub struct ViewBackedCheckCashSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub check_key: Uint256,
}

impl<'a, V: ApplyView> ViewBackedCheckCashSink<'a, V> {
    pub fn remove_check_entry(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) {
            let _ = self.view.erase(sle);
        }
    }
}

impl<'a, V: ApplyView> CheckCashApplySink for ViewBackedCheckCashSink<'a, V> {
    fn xrp_liquid_sufficient(&mut self) -> bool {
        true
    }
    fn record_delivered_xrp(&mut self) {}
    fn transfer_xrp(&mut self) -> Ter {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) {
            let source = sle.get_account_id(get_field_by_symbol("sfAccount"));
            let destination = sle.get_account_id(get_field_by_symbol("sfDestination"));
            let amount = sle.get_field_amount(get_field_by_symbol("sfSendMax"));
            if let Ok(Some(src_sle)) = self
                .view
                .peek(protocol::account_keylet(Uint160::from_void(source.data())))
            {
                let balance = src_sle.get_field_amount(get_field_by_symbol("sfBalance"));
                let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                    balance.xrp().drops() - amount.xrp().drops(),
                ));
                let mut obj = src_sle.clone_as_object();
                obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
                let _ = self
                    .view
                    .update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
            }
            if let Ok(Some(dst_sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
                destination.data(),
            ))) {
                let balance = dst_sle.get_field_amount(get_field_by_symbol("sfBalance"));
                let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                    balance.xrp().drops() + amount.xrp().drops(),
                ));
                let mut obj = dst_sle.clone_as_object();
                obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
                let _ = self
                    .view
                    .update(Arc::new(STLedgerEntry::from_stobject(obj, *dst_sle.key())));
            }
        }
        Ter::TES_SUCCESS
    }
    fn create_iou_trustline(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn update_destination_after_trustline_create(&mut self) {}
    fn prepare_iou_flow_limit(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn run_iou_flow(&mut self, _deliver_min_present: bool) -> CheckCashIouFlowResult {
        CheckCashIouFlowResult {
            ter: Ter::TES_SUCCESS,
            meets_requested_amount: true,
            meets_deliver_min: true,
        }
    }
    fn record_delivered_iou(&mut self) {}
    fn reload_check_after_iou_flow(&mut self) {}
    fn restore_iou_flow_limit(&mut self) {}
    fn remove_destination_dir(&mut self) -> bool {
        true
    }
    fn remove_owner_dir(&mut self) -> bool {
        true
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
    fn erase_check(&mut self) {
        self.remove_check_entry();
    }
    fn apply_view(&mut self) {}
}

pub struct ViewBackedPaymentChannelCreateSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub dst_account: AccountID,
    pub amount: XRPAmount,
    pub settle_delay: u32,
    pub public_key: protocol::STBlob,
    pub cancel_after: Option<u32>,
    pub destination_tag: Option<u32>,
    pub source_tag: Option<u32>,
    pub channel_key: Uint256,
}

impl<'a, V: ApplyView> PaymentChannelCreateApplySink for ViewBackedPaymentChannelCreateSink<'a, V> {
    fn create_payment_channel_entry(&mut self, _seq: bool) {
        let mut sle = STLedgerEntry::new(protocol::unchecked_keylet(self.channel_key));
        sle.set_account_id(get_field_by_symbol("sfAccount"), self.account);
        sle.set_account_id(get_field_by_symbol("sfDestination"), self.dst_account);
        sle.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(self.amount),
        );
        let _ = self.view.insert(Arc::new(sle));
    }
    fn insert_owner_directory(&mut self) -> Option<u64> {
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            self.channel_key,
            &|_| {},
        )
        .ok()
        .flatten()
    }
    fn set_owner_node(&mut self, page: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.set_field_u64(get_field_by_symbol("sfOwnerNode"), page);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn insert_destination_directory(&mut self) -> Option<u64> {
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.dst_account.data())),
            self.channel_key,
            &|_| {},
        )
        .ok()
        .flatten()
    }
    fn set_destination_node(&mut self, page: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.set_field_u64(get_field_by_symbol("sfDestinationNode"), page);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn deduct_owner_balance(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
            let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                balance.xrp().drops() - self.amount.drops(),
            ));
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
    fn update_owner_account(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let _ = self.view.update(sle);
        }
    }
}

pub struct ViewBackedPaymentChannelFundSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub channel_key: Uint256,
}

impl<'a, V: ApplyView> PaymentChannelFundApplySink<u32>
    for ViewBackedPaymentChannelFundSink<'a, V>
{
    fn update_expiration(&mut self, _expiration: u32) {}
    fn set_channel_amount(&mut self, amount_drops: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(amount_drops as i64)),
            );
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn persist_channel(&mut self) {}
    fn subtract_owner_balance(&mut self, amount_drops: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            let balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
            let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                balance.xrp().drops() - amount_drops as i64,
            ));
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn persist_owner(&mut self) {}
}

impl<'a, V: ApplyView> PaymentChannelCloseSink for ViewBackedPaymentChannelFundSink<'a, V> {
    fn remove_source_owner_directory(&mut self) -> Ter {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            let owner_node = sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
            if dir_remove(
                self.view,
                &protocol::owner_dir_keylet(Uint160::from_void(owner.data())),
                owner_node,
                *sle.key(),
                false,
            )
            .is_ok()
            {
                return Ter::TES_SUCCESS;
            }
        }
        Ter::TEF_BAD_LEDGER
    }
    fn remove_destination_owner_directory(&mut self) -> Ter {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let dst = sle.get_account_id(get_field_by_symbol("sfDestination"));
            if sle.has_field(get_field_by_symbol("sfDestinationNode")) {
                let dst_node = sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
                if dir_remove(
                    self.view,
                    &protocol::owner_dir_keylet(Uint160::from_void(dst.data())),
                    dst_node,
                    *sle.key(),
                    false,
                )
                .is_ok()
                {
                    return Ter::TES_SUCCESS;
                }
            }
        }
        Ter::TEF_BAD_LEDGER
    }
    fn source_account_exists(&mut self) -> bool {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            return self
                .view
                .exists(protocol::account_keylet(Uint160::from_void(owner.data())))
                .unwrap_or(false);
        }
        false
    }
    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            if let Ok(Some(owner_sle)) = self
                .view
                .peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                let balance = owner_sle.get_field_amount(get_field_by_symbol("sfBalance"));
                let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                    balance.xrp().drops() + refund_drops as i64,
                ));
                let mut obj = owner_sle.clone_as_object();
                obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
                let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *owner_sle.key(),
                )));
            }
        }
    }
    fn adjust_source_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            if let Ok(Some(owner_sle)) = self
                .view
                .peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                let _ = adjust_owner_count(self.view, &owner_sle, delta);
            }
        }
    }
    fn erase_channel(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let _ = self.view.erase(sle);
        }
    }
}

pub struct ViewBackedPaymentChannelClaimSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub channel_key: Uint256,
}

impl<'a, V: ApplyView> PaymentChannelClaimApplySink<u32>
    for ViewBackedPaymentChannelClaimSink<'a, V>
{
    fn remove_source_owner_directory(&mut self) -> Ter {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            let owner_node = sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
            if dir_remove(
                self.view,
                &protocol::owner_dir_keylet(Uint160::from_void(owner.data())),
                owner_node,
                *sle.key(),
                false,
            )
            .is_ok()
            {
                return Ter::TES_SUCCESS;
            }
        }
        Ter::TEF_BAD_LEDGER
    }
    fn remove_destination_owner_directory(&mut self) -> Ter {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let dst = sle.get_account_id(get_field_by_symbol("sfDestination"));
            if sle.has_field(get_field_by_symbol("sfDestinationNode")) {
                let dst_node = sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
                if dir_remove(
                    self.view,
                    &protocol::owner_dir_keylet(Uint160::from_void(dst.data())),
                    dst_node,
                    *sle.key(),
                    false,
                )
                .is_ok()
                {
                    return Ter::TES_SUCCESS;
                }
            }
        }
        Ter::TEF_BAD_LEDGER
    }
    fn source_account_exists(&mut self) -> bool {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let owner = sle.get_account_id(get_field_by_symbol("sfAccount"));
            return self
                .view
                .exists(protocol::account_keylet(Uint160::from_void(owner.data())))
                .unwrap_or(false);
        }
        false
    }
    fn apply_refund_to_source_account(&mut self, _drops: u64) {}
    fn adjust_source_owner_count(&mut self, _delta: i32) {}
    fn erase_channel(&mut self) {}
    fn destination_exists(&mut self) -> bool {
        true
    }
    fn verify_deposit_preauth(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn set_channel_balance(&mut self, balance_drops: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(
                get_field_by_symbol("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(balance_drops as i64)),
            );
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn add_destination_balance(&mut self, delta_drops: u64) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let dst = sle.get_account_id(get_field_by_symbol("sfDestination"));
            if let Ok(Some(dst_sle)) = self
                .view
                .peek(protocol::account_keylet(Uint160::from_void(dst.data())))
            {
                let balance = dst_sle.get_field_amount(get_field_by_symbol("sfBalance"));
                let new_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(
                    balance.xrp().drops() + delta_drops as i64,
                ));
                let mut obj = dst_sle.clone_as_object();
                obj.set_field_amount(get_field_by_symbol("sfBalance"), new_balance);
                let _ = self
                    .view
                    .update(Arc::new(STLedgerEntry::from_stobject(obj, *dst_sle.key())));
            }
        }
    }
    fn persist_destination_balance(&mut self) {}
    fn persist_channel_balance(&mut self) {}
    fn clear_expiration(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfExpiration"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_expiration(&mut self, expiration: u32) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.channel_key)) {
            let mut obj = sle.clone_as_object();
            obj.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
}
