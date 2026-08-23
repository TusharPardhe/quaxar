//! Check and PayChan transactor apply bridge.

use basics::math::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, adjust_owner_count, dir_insert, dir_remove};
use protocol::{
    AccountID, Asset, LedgerEntryType, STAmount, STLedgerEntry, STObject, Ter, XRPAmount,
    get_field_by_symbol,
};
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
            &ledger::describe_owner_dir(self.dst_account),
        )
        .ok()
        .flatten()
    }
    fn insert_owner_dir(&mut self) -> Option<u64> {
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            self.check_key,
            &ledger::describe_owner_dir(self.account),
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
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return true;
        };
        check_sle.get_account_id(get_field_by_symbol("sfAccount"))
            == check_sle.get_account_id(get_field_by_symbol("sfDestination"))
    }
    fn remove_destination_dir(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let page = check_sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
        dir_remove(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(destination.data())),
            page,
            self.check_key,
            true,
        )
        .unwrap_or(false)
    }
    fn remove_owner_dir(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let page = check_sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
        dir_remove(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            page,
            self.check_key,
            true,
        )
        .unwrap_or(false)
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
    // The one transaction field that was supplied: sfAmount or sfDeliverMin.
    // It is required to compute rippled's xrpDeliver and IOU Flow request;
    // sfSendMax is only the Check's upper bound, never the unconditional
    // transfer amount.
    pub requested_amount: STAmount,
    pub deliver_min_present: bool,
    // The CheckCash IOU path raises the destination-side line limit only while
    // Flow runs. Keep the exact ledger key, side, and original amount so it is
    // restored even when Flow returns a failure TER.
    pub iou_limit_override: Option<(Uint256, bool, STAmount)>,
}

impl<'a, V: ApplyView> ViewBackedCheckCashSink<'a, V> {
    pub fn remove_check_entry(&mut self) {
        if let Ok(Some(sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) {
            let _ = self.view.erase(sle);
        }
    }

    /// Match CheckCash::doApply's native branch exactly: xrpLiquid uses the
    /// released Check reserve and xrpDeliver is Amount, or the DeliverMin
    /// partial-payment expression when sfDeliverMin was supplied.
    fn xrp_delivery(
        &self,
        source: &AccountID,
        send_max: &STAmount,
    ) -> Result<(XRPAmount, XRPAmount), Ter> {
        if !send_max.native() || !self.requested_amount.native() {
            return Err(Ter::TEF_INTERNAL);
        }
        let liquid = ledger::apply_view::xrp_liquid(&*self.view, source, -1)
            .map_err(|_| Ter::TEF_BAD_LEDGER)?;
        let deliver = if self.deliver_min_present {
            XRPAmount::from_drops(
                self.requested_amount
                    .xrp()
                    .drops()
                    .max(send_max.xrp().drops().min(liquid.drops())),
            )
        } else {
            self.requested_amount.xrp()
        };
        Ok((liquid, deliver))
    }
}

impl<'a, V: ApplyView> CheckCashApplySink for ViewBackedCheckCashSink<'a, V> {
    fn xrp_liquid_sufficient(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let send_max = check_sle.get_field_amount(get_field_by_symbol("sfSendMax"));

        // CheckCash deletes this object immediately after payment. Match
        // rippled's xrpLiquid(psb, srcId, -1), then compare against the
        // Amount/DeliverMin-derived xrpDeliver rather than sfSendMax.
        self.xrp_delivery(&source, &send_max)
            .is_ok_and(|(liquid, deliver)| liquid >= deliver)
    }

    fn record_delivered_xrp(&mut self) {}

    fn transfer_xrp(&mut self) -> Ter {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return Ter::TEC_FAILED_PROCESSING;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let send_max = check_sle.get_field_amount(get_field_by_symbol("sfSendMax"));

        // Enforce the reserve-aware calculation again immediately before
        // mutation. The generic shell checks this too, but no caller may
        // bypass xrpLiquid by calling transfer_xrp directly.
        let (liquid, deliver) = match self.xrp_delivery(&source, &send_max) {
            Ok(values) => values,
            Err(ter) => return ter,
        };
        if liquid < deliver {
            return Ter::TEC_UNFUNDED_PAYMENT;
        }

        ledger::ripple_state_helpers::transfer_xrp(self.view, &source, &destination, deliver)
    }

    fn create_iou_trustline(&mut self) -> Ter {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let send_max = check_sle.get_field_amount(get_field_by_symbol("sfSendMax"));
        let Asset::Issue(issue) = send_max.asset() else {
            // This hook is specifically the IOU trust-line branch. MPT setup
            // is a distinct CheckCash path and must not be represented as an
            // IOU RippleState.
            return Ter::TEC_NO_LINE;
        };
        if issue.native() {
            return Ter::TEF_BAD_LEDGER;
        }

        // rippled uses the destination unless it is the issuer, in which case
        // the source's existing issuer line is the relevant line.
        let truster = if issue.account == destination {
            source
        } else {
            destination
        };
        let line_keylet = protocol::line(truster, issue.account, issue.currency);
        match self.view.exists(line_keylet) {
            Ok(true) => return Ter::TES_SUCCESS,
            Ok(false) => {}
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }

        // CheckCash creates the line at the cashing destination's expense.
        // The comparison deliberately uses the current view's balance: this
        // bridge applies after transaction fees, so it is the only balance
        // available here and must satisfy the post-create reserve.
        let destination_keylet = protocol::account_keylet(Uint160::from_void(destination.data()));
        let Ok(Some(destination_sle)) = self.view.peek(destination_keylet) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let owner_count = destination_sle.get_field_u32(get_field_by_symbol("sfOwnerCount"));
        let required_reserve = self.view.fees().account_reserve(owner_count as usize + 1) as i64;
        if destination_sle
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops()
            < required_reserve
        {
            return Ter::TEC_NO_LINE_INSUF_RESERVE;
        }

        // trustCreate(psb, destLow, issuer, destination, ...) creates a
        // zero-balance RippleState, reserves the destination side, and sets
        // that side's NoRipple flag when the destination has DefaultRipple
        // disabled. Both owner-directory page numbers are ledger fields.
        let destination_low = issue.account > destination;
        let low_account = if destination_low {
            destination
        } else {
            issue.account
        };
        let high_account = if destination_low {
            issue.account
        } else {
            destination
        };

        let low_dir = protocol::owner_dir_keylet(Uint160::from_void(low_account.data()));
        let low_node = match dir_insert(
            self.view,
            &low_dir,
            line_keylet.key,
            &ledger::describe_owner_dir(low_account),
        ) {
            Ok(Some(page)) => page,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let high_dir = protocol::owner_dir_keylet(Uint160::from_void(high_account.data()));
        let high_node = match dir_insert(
            self.view,
            &high_dir,
            line_keylet.key,
            &ledger::describe_owner_dir(high_account),
        ) {
            Ok(Some(page)) => page,
            Ok(None) => {
                let _ = dir_remove(self.view, &low_dir, low_node, line_keylet.key, false);
                return Ter::TEC_DIR_FULL;
            }
            Err(_) => {
                let _ = dir_remove(self.view, &low_dir, low_node, line_keylet.key, false);
                return Ter::TEF_BAD_LEDGER;
            }
        };

        let mut balance = send_max.zeroed();
        balance.set_issuer(protocol::no_account());
        let mut low_limit = send_max.zeroed();
        low_limit.set_issuer(low_account);
        let mut high_limit = send_max.zeroed();
        high_limit.set_issuer(high_account);

        let mut flags = if destination_low {
            0x0001_0000
        } else {
            0x0002_0000
        };
        if destination_sle.get_field_u32(get_field_by_symbol("sfFlags"))
            & protocol::lsfDefaultRipple
            == 0
        {
            flags |= if destination_low {
                0x0010_0000
            } else {
                0x0020_0000
            };
        }

        let mut line = STObject::new(get_field_by_symbol("sfGeneric"));
        line.set_field_u16(
            get_field_by_symbol("sfLedgerEntryType"),
            LedgerEntryType::RippleState as u16,
        );
        line.set_field_amount(get_field_by_symbol("sfBalance"), balance);
        line.set_field_amount(get_field_by_symbol("sfLowLimit"), low_limit);
        line.set_field_amount(get_field_by_symbol("sfHighLimit"), high_limit);
        line.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        line.set_field_u64(get_field_by_symbol("sfLowNode"), low_node);
        line.set_field_u64(get_field_by_symbol("sfHighNode"), high_node);

        let line_sle = Arc::new(STLedgerEntry::from_stobject(line, line_keylet.key));
        if self.view.insert(line_sle.clone()).is_err() {
            let _ = dir_remove(self.view, &high_dir, high_node, line_keylet.key, false);
            let _ = dir_remove(self.view, &low_dir, low_node, line_keylet.key, false);
            return Ter::TEF_BAD_LEDGER;
        }
        if adjust_owner_count(self.view, &destination_sle, 1).is_err() {
            let _ = self.view.erase(line_sle);
            let _ = dir_remove(self.view, &high_dir, high_node, line_keylet.key, false);
            let _ = dir_remove(self.view, &low_dir, low_node, line_keylet.key, false);
            return Ter::TEF_BAD_LEDGER;
        }
        Ter::TES_SUCCESS
    }

    fn update_destination_after_trustline_create(&mut self) {
        // create_iou_trustline persists the destination owner-count mutation;
        // this trait phase exists to mirror rippled's explicit psb.update.
    }

    fn trustline_available_after_create(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let send_max = check_sle.get_field_amount(get_field_by_symbol("sfSendMax"));
        let Asset::Issue(issue) = send_max.asset() else {
            return false;
        };
        let truster = if issue.account == destination {
            source
        } else {
            destination
        };
        self.view
            .exists(protocol::line(truster, issue.account, issue.currency))
            .unwrap_or(false)
    }

    fn prepare_iou_flow_limit(&mut self) -> Ter {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let send_max = check_sle.get_field_amount(get_field_by_symbol("sfSendMax"));
        let Asset::Issue(issue) = send_max.asset() else {
            return Ter::TEC_NO_LINE;
        };
        let truster = if issue.account == destination {
            source
        } else {
            destination
        };
        let line_keylet = protocol::line(truster, issue.account, issue.currency);
        let Ok(Some(line_sle)) = self.view.peek(line_keylet) else {
            return Ter::TEC_NO_LINE;
        };

        // The destination signed the CheckCash transaction, so rippled permits
        // Flow to deliver beyond its configured limit for this one call. Keep
        // the original amount to restore unconditionally after Flow returns.
        let destination_low = issue.account > destination;
        let limit_field = if destination_low {
            get_field_by_symbol("sfLowLimit")
        } else {
            get_field_by_symbol("sfHighLimit")
        };
        let saved_limit = line_sle.get_field_amount(limit_field);
        let max_limit = protocol::to_max_amount::<STAmount>(send_max.asset());
        let mut line = line_sle.clone_as_object();
        line.set_field_amount(limit_field, max_limit);
        if self
            .view
            .update(Arc::new(STLedgerEntry::from_stobject(
                line,
                *line_sle.key(),
            )))
            .is_err()
        {
            return Ter::TEF_BAD_LEDGER;
        }
        self.iou_limit_override = Some((*line_sle.key(), destination_low, saved_limit));
        Ter::TES_SUCCESS
    }

    fn run_iou_flow(&mut self, _deliver_min_present: bool) -> CheckCashIouFlowResult {
        CheckCashIouFlowResult {
            ter: Ter::TEC_NO_LINE,
            meets_requested_amount: false,
            meets_deliver_min: false,
        }
    }
    fn record_delivered_iou(&mut self) {}
    fn reload_check_after_iou_flow(&mut self) {}

    fn restore_iou_flow_limit(&mut self) {
        let Some((line_key, destination_low, saved_limit)) = self.iou_limit_override.take() else {
            return;
        };
        let Ok(Some(line_sle)) = self.view.peek(protocol::unchecked_keylet(line_key)) else {
            return;
        };
        let limit_field = if destination_low {
            get_field_by_symbol("sfLowLimit")
        } else {
            get_field_by_symbol("sfHighLimit")
        };
        let mut line = line_sle.clone_as_object();
        line.set_field_amount(limit_field, saved_limit);
        let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
            line,
            *line_sle.key(),
        )));
    }

    fn remove_destination_dir(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let destination = check_sle.get_account_id(get_field_by_symbol("sfDestination"));
        let page = check_sle.get_field_u64(get_field_by_symbol("sfDestinationNode"));
        dir_remove(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(destination.data())),
            page,
            self.check_key,
            true,
        )
        .unwrap_or(false)
    }

    fn remove_owner_dir(&mut self) -> bool {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return false;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let page = check_sle.get_field_u64(get_field_by_symbol("sfOwnerNode"));
        dir_remove(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(source.data())),
            page,
            self.check_key,
            true,
        )
        .unwrap_or(false)
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        let Ok(Some(check_sle)) = self.view.peek(protocol::unchecked_keylet(self.check_key)) else {
            return;
        };
        let source = check_sle.get_account_id(get_field_by_symbol("sfAccount"));
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(Uint160::from_void(source.data())))
        {
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
            &ledger::describe_owner_dir(self.account),
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
            &ledger::describe_owner_dir(self.dst_account),
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
