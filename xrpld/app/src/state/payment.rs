//! Full Payment transactor — reference the reference implementation parity.
//!
//! Handles:
//! - Direct XRP-to-XRP payments (with account creation)
//! - IOU/path payments via RippleCalc (flow engine)
//! - MPT direct payments (without flow engine, pre-MPTokensV2)
//! - Partial payments (tfPartialPayment)
//! - Limit quality (tfLimitQuality)
//! - DeliverMin checking
//! - Deposit preauth
//! - Destination tag requirement
//! - Reserve checks
//! - Pseudo-account rejection

use std::{cell::RefCell, sync::Arc};

use basics::math::base_uint::Uint160;
use protocol::{
    AccountID, Asset, PARITY_RATE, STAmount, STLedgerEntry, STTx, Ter, XRPAmount, divide_rate,
    equal_tokens, get_field_by_symbol, is_ter_retry, is_tes_success, multiply_rate,
};
thread_local! {
    static DELIVERED_AMOUNT_CAPTURES: RefCell<Vec<Option<STAmount>>> = const { RefCell::new(Vec::new()) };
}

/// Scoped equivalent of rippled's `ApplyContext::deliver` metadata handoff.
/// Transaction-shell callers that do not construct metadata safely discard the
/// capture when their application returns.
pub struct DeliveredAmountCapture {
    active: bool,
}

impl Default for DeliveredAmountCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveredAmountCapture {
    pub fn new() -> Self {
        DELIVERED_AMOUNT_CAPTURES.with(|captures| captures.borrow_mut().push(None));
        Self { active: true }
    }

    pub fn finish(mut self) -> Option<STAmount> {
        self.active = false;
        DELIVERED_AMOUNT_CAPTURES.with(|captures| captures.borrow_mut().pop().flatten())
    }
}

impl Drop for DeliveredAmountCapture {
    fn drop(&mut self) {
        if self.active {
            DELIVERED_AMOUNT_CAPTURES.with(|captures| {
                let _ = captures.borrow_mut().pop();
            });
        }
    }
}

pub(crate) fn record_delivered_amount(amount: STAmount) {
    DELIVERED_AMOUNT_CAPTURES.with(|captures| {
        if let Some(delivered_amount) = captures.borrow_mut().last_mut() {
            *delivered_amount = Some(amount);
        }
    });
}

fn should_record_path_delivered_amount(
    result: Ter,
    actual_amount_out: &STAmount,
    destination_amount: &STAmount,
) -> bool {
    is_tes_success(result) && actual_amount_out != destination_amount
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

const TF_PARTIAL_PAYMENT: u32 = 0x0002_0000;
const TF_NO_RIPPLE_DIRECT: u32 = 0x0001_0000;
const TF_LIMIT_QUALITY: u32 = 0x0004_0000;

const LSF_REQUIRE_DEST_TAG: u32 = 0x0002_0000;
const LSF_PASSWORD_SPENT: u32 = 0x0001_0000;
// Kept for compatibility with the reference source flag checks; the current Rust path does
// not yet consume the trustline-auth branch directly.
#[allow(dead_code)]
const LSF_REQUIRE_AUTH: u32 = 0x0004_0000;

fn is_redundant_self_payment(
    account: AccountID,
    destination: AccountID,
    source_amount: &STAmount,
    destination_amount: &STAmount,
    has_paths: bool,
) -> bool {
    account == destination
        && equal_tokens(source_amount.asset(), destination_amount.asset())
        && !has_paths
}

/// Full reference Payment::doApply parity.
///
/// Called from `handle_real_dispatch` with the view, transaction, and pre-fee balance.
pub fn do_payment<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let dst_account_id = sttx.get_account_id(sf("sfDestination"));

    let dst_amount = sttx.get_field_amount(sf("sfAmount"));
    let tx_flags = sttx.get_field_u32(sf("sfFlags"));
    let has_paths = sttx.is_field_present(sf("sfPaths"));
    let send_max = if sttx.is_field_present(sf("sfSendMax")) {
        Some(sttx.get_field_amount(sf("sfSendMax")))
    } else {
        None
    };
    let deliver_min = if sttx.is_field_present(sf("sfDeliverMin")) {
        Some(sttx.get_field_amount(sf("sfDeliverMin")))
    } else {
        None
    };

    // rippled Payment::preflight rejects a nonpositive destination amount and
    // a present nonpositive SendMax before any path/flow arithmetic.  Without
    // this boundary an invalid issuer-source partial payment can enter
    // RippleCalc with a zero required input and reach a raw divide-by-zero.
    if dst_amount.signum() <= 0 || send_max.as_ref().is_some_and(|amount| amount.signum() <= 0) {
        return Ter::TEM_BAD_AMOUNT;
    }

    let partial_payment_allowed = (tx_flags & TF_PARTIAL_PAYMENT) != 0;
    let limit_quality = (tx_flags & TF_LIMIT_QUALITY) != 0;
    let default_paths_allowed = (tx_flags & TF_NO_RIPPLE_DIRECT) == 0;

    // Compute maxSourceAmount (reference getMaxSourceAmount)
    let max_source_amount = if let Some(ref sm) = send_max {
        sm.clone()
    } else if dst_amount.native() || dst_amount.holds_mpt_issue() {
        dst_amount.clone()
    } else {
        // IOU: use same mantissa/exponent but with source account as issuer
        // reference: STAmount(Issue{issue.currency, account}, dstAmount.mantissa(), dstAmount.exponent(), dstAmount < zero)
        let mut sa = dst_amount.clone();
        sa.set_issuer(account);
        sa
    };

    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst_account_id.data()));

    // Match rippled Payment::preflight: a payment to self is redundant only
    // when source and destination assets are identical and no explicit path
    // can perform an arbitrage/conversion. Cross-currency self-payments must
    // continue into Flow; rejecting them here as tecPATH_DRY forks valid
    // mainnet ledger state.
    if is_redundant_self_payment(
        account,
        dst_account_id,
        &max_source_amount,
        &dst_amount,
        has_paths,
    ) {
        return Ter::TEM_REDUNDANT;
    }

    // Peek destination account
    let dst_exists = view.peek(dst_keylet).ok().flatten();

    match dst_exists.as_ref() {
        None => {
            // Destination account does not exist
            if !dst_amount.native() {
                // Can't create account with IOU
                return Ter::TEC_NO_DST;
            }
            // Can't create account with partial payment
            if view.open() && partial_payment_allowed {
                return Ter::TEL_NO_DST_PARTIAL;
            }
            // Check minimum reserve for account creation
            let reserve = view.fees().reserve;
            if dst_amount.xrp().drops() < reserve as i64 {
                static NO_DST_LOG: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                if NO_DST_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
                    tracing::debug!(target: "tx",
                        "[payment_debug] NO_DST_INSUF_XRP: amount_drops={} reserve={} partial={}",
                        dst_amount.xrp().drops(),
                        reserve,
                        partial_payment_allowed
                    );
                }
                return Ter::TEC_NO_DST_INSUF_XRP;
            }
        }
        Some(dst_sle) => {
            let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));

            // Check DestinationTag requirement
            if (dst_flags & LSF_REQUIRE_DEST_TAG) != 0
                && !sttx.is_field_present(sf("sfDestinationTag"))
            {
                return Ter::TEC_DST_TAG_NEEDED;
            }
        }
    }

    // Determine if this is a "ripple" payment (IOU/path) or direct XRP.
    // In C++, MPTokensV1 direct payments bypass RippleCalc and are handled by
    // Payment::doApply's direct-MPT branch. MPTokensV2 routes through the
    // payment engine.
    let is_dst_native = dst_amount.native();
    let is_dst_mpt = dst_amount.holds_mpt_issue();
    let mp_tokens_v2 = view.rules().enabled(&protocol::feature_id("MPTokensV2"));
    let ripple =
        (has_paths || send_max.is_some() || !is_dst_native) && (!is_dst_mpt || mp_tokens_v2);

    if ripple {
        // C++ parity: ALL IOU payments go through rippleCalc.
        // No direct shortcut — the flow engine handles transfer rates,
        // path finding, and multi-hop payments correctly.

        // IOU/path payment via RippleCalc

        // Debug: log payment type for diagnosis
        static PAY_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let c = PAY_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if c < 50 {
            tracing::debug!(target: "tx",
                "[payment_debug] src_native={} dst_native={} has_paths={} has_sendmax={} sendmax_native={}",
                max_source_amount.native(),
                dst_amount.native(),
                has_paths,
                send_max.is_some(),
                send_max.as_ref().map(|s| s.native()).unwrap_or(false),
            );
        }

        // Deposit preauth check for IOU payments
        if let Some(ref dst_sle) = dst_exists {
            if let Some(ter) =
                verify_deposit_preauth(view, sttx, &account, &dst_account_id, dst_sle)
            {
                return ter;
            }
        }

        let input = ledger::ripple_calc::RippleCalcInput {
            partial_payment_allowed,
            default_paths_allowed,
            limit_quality,
            is_ledger_open: view.open(),
        };

        let paths = if has_paths {
            sttx.get_field_path_set(sf("sfPaths"))
        } else {
            protocol::STPathSet::new(sf("sfPaths"))
        };

        match ledger::ripple_calc::ripple_calculate(
            view,
            &max_source_amount,
            &dst_amount,
            &dst_account_id,
            &account,
            &paths,
            &input,
        ) {
            Ok(output) => {
                let mut result = output.result;

                // reference: if success and actualAmountOut != dstAmount
                if is_tes_success(result) {
                    if output.actual_amount_out != dst_amount {
                        if output.actual_amount_out.signum() <= 0 {
                            result = Ter::TEC_PATH_DRY;
                        } else if let Some(ref dmin) = deliver_min {
                            if &output.actual_amount_out < dmin {
                                result = Ter::TEC_PATH_PARTIAL;
                            }
                        } else if !partial_payment_allowed {
                            result = Ter::TEC_PATH_PARTIAL;
                        }
                        // else: partial payment allowed, deliver what we can
                    }
                }

                // reference: if isTerRetry(terResult) terResult = tecPATH_DRY
                if is_ter_retry(result) {
                    result = Ter::TEC_PATH_DRY;
                }
                if should_record_path_delivered_amount(
                    result,
                    &output.actual_amount_out,
                    &dst_amount,
                ) {
                    // `Payment::doApply` hands Flow's actual output to
                    // ApplyContext only when it differs from Amount, so
                    // TxMeta records DeliveredAmount for successful partial
                    // payments without changing exact-payment metadata.
                    record_delivered_amount(output.actual_amount_out);
                }

                result
            }
            Err(_) => Ter::TEF_INTERNAL,
        }
    } else if is_dst_mpt {
        if let Some(ref dst_sle) = dst_exists {
            if let Some(ter) =
                verify_deposit_preauth(view, sttx, &account, &dst_account_id, dst_sle)
            {
                return ter;
            }
        }
        do_direct_mpt_payment(
            view,
            &account,
            &dst_account_id,
            &dst_amount,
            &max_source_amount,
            deliver_min.as_ref(),
            partial_payment_allowed,
            dst_exists,
        )
    } else {
        // Direct XRP payment
        do_direct_xrp_payment(
            view,
            &account,
            &dst_account_id,
            &dst_amount,
            dst_exists,
            pre_fee_balance_drops,
            sttx,
        )
    }
}

fn do_direct_mpt_payment<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    dst_account_id: &AccountID,
    dst_amount: &STAmount,
    max_source_amount: &STAmount,
    deliver_min: Option<&STAmount>,
    partial_payment_allowed: bool,
    dst_sle_opt: Option<Arc<STLedgerEntry>>,
) -> Ter {
    let Asset::MPTIssue(mpt_issue) = dst_amount.asset() else {
        return Ter::TEF_INTERNAL;
    };

    let Ok(auth) = ledger::mptoken_helpers::require_auth_mpt(view, &mpt_issue, account) else {
        return Ter::TEF_BAD_LEDGER;
    };
    if !is_tes_success(auth) {
        return auth;
    }
    let Ok(auth) = ledger::mptoken_helpers::require_auth_mpt(view, &mpt_issue, dst_account_id)
    else {
        return Ter::TEF_BAD_LEDGER;
    };
    if !is_tes_success(auth) {
        return auth;
    }
    let Ok(transfer) =
        ledger::mptoken_helpers::can_transfer_mpt(view, &mpt_issue, account, dst_account_id)
    else {
        return Ter::TEF_BAD_LEDGER;
    };
    if !is_tes_success(transfer) {
        return transfer;
    }

    let issuer = mpt_issue.issuer();
    let mut rate = PARITY_RATE;
    if account != &issuer && dst_account_id != &issuer {
        if ledger::mptoken_helpers::is_frozen_mpt(view, account, &mpt_issue).unwrap_or(true)
            || ledger::mptoken_helpers::is_frozen_mpt(view, dst_account_id, &mpt_issue)
                .unwrap_or(true)
        {
            return Ter::TEC_LOCKED;
        }
        rate = ledger::mptoken_helpers::transfer_rate_mpt(view, mpt_issue.mpt_id())
            .unwrap_or(PARITY_RATE);
    }

    let mut amount_deliver = dst_amount.clone();
    let mut required_max_source_amount = multiply_rate(dst_amount, rate);
    if partial_payment_allowed && required_max_source_amount > *max_source_amount {
        required_max_source_amount = max_source_amount.clone();
        amount_deliver = divide_rate(max_source_amount, rate);
    }
    if required_max_source_amount > *max_source_amount
        || deliver_min.is_some_and(|deliver_min| amount_deliver < *deliver_min)
    {
        return Ter::TEC_PATH_PARTIAL;
    }

    if dst_account_id != &issuer
        && view
            .peek(protocol::mptoken_keylet_from_mptid(
                mpt_issue.mpt_id(),
                Uint160::from_void(dst_account_id.data()),
            ))
            .ok()
            .flatten()
            .is_none()
    {
        let Some(dst_sle) = dst_sle_opt else {
            return Ter::TEC_NO_PERMISSION;
        };
        let owner_count = dst_sle.get_field_u32(sf("sfOwnerCount"));
        let balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        if balance < view.fees().account_reserve(owner_count as usize + 1) as i64 {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }
        match ledger::mptoken_helpers::check_create_mpt(view, &mpt_issue, dst_account_id) {
            Ok(Ter::TES_SUCCESS | Ter::TEC_DUPLICATE) => {}
            Ok(ter) => return ter,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }

    let mut result =
        ledger::ripple_state_helpers::account_send(view, account, dst_account_id, &amount_deliver);
    if matches!(result, Ter::TEC_INSUFFICIENT_FUNDS | Ter::TEC_PATH_DRY) {
        result = Ter::TEC_PATH_PARTIAL;
    }
    if is_tes_success(result)
        && view.rules().enabled(&protocol::fix_mpt_delivered_amount())
        && amount_deliver != *dst_amount
    {
        // Matches Payment.cpp: direct MPT payments use the actual net amount,
        // not the requested destination amount, when the amendment is active.
        record_delivered_amount(amount_deliver);
    }
    result
}

#[allow(dead_code)] // reserve for M7 sweep
fn is_direct_iou_payment(
    dst_amount: &STAmount,
    send_max: Option<&STAmount>,
    has_paths: bool,
    account: &AccountID,
    dst_account_id: &AccountID,
) -> bool {
    // Only use the direct IOU when one party is the issuer.
    // Non-issuer to non-issuer payments MUST go through rippleCalculate
    // which builds the default path (sender → issuer → destination).
    if has_paths || dst_amount.native() {
        return false;
    }
    let issue = dst_amount.issue();
    let one_is_issuer = *account == issue.account || *dst_account_id == issue.account;
    if !one_is_issuer {
        return false;
    }
    send_max
        .is_some_and(|send_max| send_max.asset() == dst_amount.asset() && send_max == dst_amount)
}

#[allow(dead_code)] // reserve for M7 sweep
fn do_direct_iou_payment<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    dst_account_id: &AccountID,
    dst_amount: &STAmount,
    partial_payment_allowed: bool,
) -> Ter {
    let issue = dst_amount.issue();

    // C++ parity: check if the trust line is frozen before allowing transfer.
    // Individual freeze (issuer froze this account's line) or global freeze.
    if *account != issue.account && *dst_account_id != issue.account {
        // Neither party is the issuer — check both sides for freeze
        if ledger::ripple_state_helpers::is_frozen(view, account, &issue)
            || ledger::ripple_state_helpers::is_frozen(view, dst_account_id, &issue)
        {
            return Ter::TEC_PATH_DRY;
        }
    } else if *account != issue.account {
        if ledger::ripple_state_helpers::is_frozen(view, account, &issue) {
            return Ter::TEC_PATH_DRY;
        }
    } else if *dst_account_id != issue.account {
        if ledger::ripple_state_helpers::is_frozen(view, dst_account_id, &issue) {
            return Ter::TEC_PATH_DRY;
        }
    }

    let available = if *account == issue.account {
        dst_amount.clone()
    } else {
        let mut balance = ledger::ripple_state_helpers::account_holds(
            view,
            account,
            &issue.account,
            issue.currency,
        );
        if balance.asset() != dst_amount.asset() {
            balance = dst_amount.zeroed();
        }
        balance
    };

    static DIRECT_IOU_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if DIRECT_IOU_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 30 {
        tracing::debug!(target: "tx",
            "[direct_iou] available_signum={} dst_signum={} partial={} src_is_issuer={}",
            available.signum(),
            dst_amount.signum(),
            partial_payment_allowed,
            *account == issue.account
        );
    }

    let amount_deliver = if available < *dst_amount {
        if !partial_payment_allowed || available.signum() <= 0 {
            return Ter::TEC_PATH_PARTIAL;
        }
        available
    } else {
        dst_amount.clone()
    };

    let mut result = ledger::ripple_state_helpers::account_send_with_fee(
        view,
        account,
        dst_account_id,
        &amount_deliver,
    );
    if matches!(result, Ter::TEC_PATH_DRY | Ter::TEC_UNFUNDED_PAYMENT) {
        result = Ter::TEC_PATH_PARTIAL;
    }
    result
}

fn do_direct_xrp_payment<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    dst_account_id: &AccountID,
    dst_amount: &STAmount,
    dst_sle_opt: Option<Arc<STLedgerEntry>>,
    pre_fee_balance_drops: Option<i64>,
    sttx: &STTx,
) -> Ter {
    let xrp_drops = dst_amount.xrp().drops();
    if xrp_drops <= 0 {
        return Ter::TEM_BAD_AMOUNT;
    }

    let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst_account_id.data()));

    // Get or create destination account
    let dst_sle = if let Some(existing) = dst_sle_opt {
        // Tell the engine we intend to change the destination
        existing
    } else {
        // Create the account — reference creates SLE with Sequence = view().seq()
        let ledger_seq = view.seq();
        let mut obj = protocol::STObject::new(sf("sfLedgerEntry"));
        obj.set_field_u16(sf("sfLedgerEntryType"), 0x0061); // ltACCOUNT_ROOT
        obj.set_account_id(sf("sfAccount"), *dst_account_id);
        obj.set_field_u32(sf("sfSequence"), ledger_seq);
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
        );
        let new_sle = Arc::new(STLedgerEntry::from_stobject(obj, dst_keylet.key));
        let _ = view.insert(new_sle.clone());
        new_sle
    };

    // Source account checks
    let Some(src_sle) = view.peek(src_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };

    // Reserve check — reference checks preFeeBalance_ < dstAmount + minRequiredFunds
    let owner_count = src_sle.get_field_u32(sf("sfOwnerCount"));
    let reserve = view.fees().account_reserve(owner_count as usize) as i64;
    let src_balance = pre_fee_balance_drops
        .unwrap_or_else(|| src_sle.get_field_amount(sf("sfBalance")).xrp().drops());

    // reference: auto const minRequiredFunds = accountIsPayer
    //        ? std::max(reserve, ctx_.tx.getFieldAmount(sfFee).xrp())
    //        : reserve;
    // For simplicity, account is always the fee payer in our current model
    let min_required_funds = reserve;

    if src_balance < xrp_drops + min_required_funds {
        return Ter::TEC_UNFUNDED_PAYMENT;
    }

    // Pseudo-account check — reference rejects payments to pseudo-accounts
    // A pseudo-account has a non-zero sfRegularKey but no sfSequence > 0 and
    // specific discriminator fields. For now we check the common case:
    // accounts with sfLedgerEntryType != ltACCOUNT_ROOT are pseudo.
    // In practice, pseudo-accounts are rare on mainnet. The reference check is:
    // if (isPseudoAccount(sleDst)) return tecNO_PERMISSION;
    // isPseudoAccount checks for AMM account (lsfAMM flag)
    let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));
    if (dst_flags & 0x0200_0000) != 0 {
        // lsfAMM = 0x02000000
        return Ter::TEC_NO_PERMISSION;
    }

    // Deposit preauth check for XRP payments
    // reference: if (dstAmount > dstReserve || sleDst->balance > dstReserve) check deposit preauth
    let dst_reserve = view.fees().reserve as i64;
    let dst_balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
    if xrp_drops > dst_reserve || dst_balance > dst_reserve {
        if let Some(ter) = verify_deposit_preauth(view, sttx, account, dst_account_id, &dst_sle) {
            return ter;
        }
    }

    // Do the arithmetic — debit source, credit destination
    let new_src_balance = src_sle.get_field_amount(sf("sfBalance")).xrp().drops() - xrp_drops;
    let mut src_obj = src_sle.clone_as_object();
    src_obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(new_src_balance)),
    );
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        src_obj,
        *src_sle.key(),
    )));

    let new_dst_balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops() + xrp_drops;
    let mut dst_obj = dst_sle.clone_as_object();
    dst_obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(new_dst_balance)),
    );
    // reference: Re-arm the password change fee if we can and need to.
    let cur_dst_flags = dst_obj.get_field_u32(sf("sfFlags"));
    if (cur_dst_flags & LSF_PASSWORD_SPENT) != 0 {
        dst_obj.set_field_u32(sf("sfFlags"), cur_dst_flags & !LSF_PASSWORD_SPENT);
    }
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        dst_obj,
        *dst_sle.key(),
    )));

    Ter::TES_SUCCESS
}

///
/// Returns `None` if deposit is allowed, `Some(Ter)` if rejected.
fn verify_deposit_preauth<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    src: &AccountID,
    dst: &AccountID,
    dst_sle: &STLedgerEntry,
) -> Option<Ter> {
    match ledger::credential_helpers::verify_deposit_preauth(sttx, view, src, dst, Some(dst_sle)) {
        Ok(Ter::TES_SUCCESS) => None,
        Ok(ter) => Some(ter),
        Err(_) => Some(Ter::TEF_BAD_LEDGER),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_redundant_self_payment, should_record_path_delivered_amount};
    use protocol::{AccountID, Issue, STAmount, Ter, XRPAmount, get_field_by_symbol};

    #[test]
    fn exact_path_payment_does_not_record_delivered_amount() {
        let requested = STAmount::from_xrp_amount(XRPAmount::from_drops(20));
        let partial = STAmount::from_xrp_amount(XRPAmount::from_drops(19));

        assert!(!should_record_path_delivered_amount(
            Ter::TES_SUCCESS,
            &requested,
            &requested,
        ));
        assert!(should_record_path_delivered_amount(
            Ter::TES_SUCCESS,
            &partial,
            &requested,
        ));
        assert!(!should_record_path_delivered_amount(
            Ter::TEC_PATH_PARTIAL,
            &partial,
            &requested,
        ));
    }

    #[test]
    fn cross_currency_self_payment_is_not_redundant() {
        let account = AccountID::from_array([0x11; 20]);
        let source_issue = Issue {
            currency: protocol::Currency::from_array([0x22; 20]),
            account: AccountID::from_array([0x33; 20]),
        };
        let destination_issue = Issue {
            currency: protocol::Currency::from_array([0x44; 20]),
            account: AccountID::from_array([0x55; 20]),
        };
        let source = STAmount::from_iou_amount(
            get_field_by_symbol("sfSendMax"),
            protocol::IOUAmount::from_parts(1, 0).expect("valid positive IOU amount"),
            source_issue,
        );
        let destination = STAmount::from_iou_amount(
            get_field_by_symbol("sfAmount"),
            protocol::IOUAmount::from_parts(1, 0).expect("valid positive IOU amount"),
            destination_issue,
        );
        assert!(!is_redundant_self_payment(
            account,
            account,
            &source,
            &destination,
            false,
        ));

        let xrp = STAmount::from_xrp_amount(XRPAmount::from_drops(1));
        assert!(is_redundant_self_payment(
            account, account, &xrp, &xrp, false,
        ));
    }

    #[test]
    fn same_currency_iou_self_payment_is_redundant_despite_issuer_change() {
        let account = AccountID::from_array([0x11; 20]);
        let source = STAmount::from_iou_amount(
            get_field_by_symbol("sfSendMax"),
            protocol::IOUAmount::from_parts(1, 0).expect("valid positive IOU amount"),
            Issue {
                currency: protocol::Currency::from_array([0x22; 20]),
                account: AccountID::from_array([0x33; 20]),
            },
        );
        let destination = STAmount::from_iou_amount(
            get_field_by_symbol("sfAmount"),
            protocol::IOUAmount::from_parts(1, 0).expect("valid positive IOU amount"),
            Issue {
                currency: protocol::Currency::from_array([0x22; 20]),
                account: AccountID::from_array([0x55; 20]),
            },
        );

        assert!(is_redundant_self_payment(
            account,
            account,
            &source,
            &destination,
            false,
        ));
        assert!(!is_redundant_self_payment(
            account,
            account,
            &source,
            &destination,
            true,
        ));
    }
}
