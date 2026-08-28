use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::{NumberParts as RuntimeNumber, get_mantissa_scale},
};
use ledger::{RelativeDistanceAmount, views::apply_view::ApplyView};
use protocol::{
    AccountID, Asset, MPTIssue, STAmount, STLedgerEntry, STNumber, STTx, Ter, XRPAmount,
    account_keylet, feature_id, get_field_by_symbol, mpt_issuance_keylet_from_mptid,
    mptoken_keylet_from_mptid,
};

pub(super) fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub(super) fn lending_protocol_dependencies_enabled<V: ApplyView>(view: &V, sttx: &STTx) -> bool {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return false;
    }
    if !view.rules().enabled(&feature_id("MPTokensV1")) {
        return false;
    }
    if sttx.is_field_present(sf("sfDomainID"))
        && !view.rules().enabled(&feature_id("PermissionedDomains"))
    {
        return false;
    }
    true
}

pub(super) fn to_160(account: &AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

pub(super) fn account_send<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    // Every lending `accountSend` call in pinned rippled passes
    // WaiveTransferFee::Yes. MPT transfer authorization, where required, is a
    // preclaim concern and must not be re-evaluated during mutation.
    account_send_with_mpt_transfer_waiver(view, sttx, from, to, amount, true)
}

/// Send one asset to multiple recipients as one canonical ledger operation.
///
/// This is the lending-only equivalent of rippled's `accountSendMulti`.  In
/// particular, MPT issuance limits and the sender debit are evaluated across
/// the complete receiver set; implementing this as repeated `accountSend`
/// calls changes both the MaximumAmount decision and the failure ordering.
pub(super) fn account_send_multi<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    asset: Asset,
    receivers: &[(AccountID, STAmount)],
) -> Ter {
    if receivers.len() < 2 || receivers.iter().any(|(_, amount)| amount.asset() != asset) {
        return Ter::TEC_INTERNAL;
    }

    match asset {
        Asset::Issue(issue) if issue.native() => {
            let sender_sle = match view.peek(account_keylet(to_160(sender))) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEF_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let mut take_from_sender = 0_i64;

            for (receiver, amount) in receivers {
                let drops = amount.xrp().drops();
                if drops < 0 {
                    return Ter::TEC_INTERNAL;
                }
                if drops == 0 || receiver == sender {
                    continue;
                }
                let receiver_sle = match view.peek(account_keylet(to_160(receiver))) {
                    Ok(Some(sle)) => sle,
                    Ok(None) => continue,
                    Err(_) => return Ter::TEF_BAD_LEDGER,
                };
                let receiver_balance = receiver_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let Some(next_balance) = receiver_balance.checked_add(drops) else {
                    return Ter::TEC_INTERNAL;
                };
                let mut updated = receiver_sle.clone_as_object();
                updated.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(next_balance)),
                );
                if view
                    .update(Arc::new(STLedgerEntry::from_stobject(
                        updated,
                        *receiver_sle.key(),
                    )))
                    .is_err()
                {
                    return Ter::TEF_BAD_LEDGER;
                }
                let Some(next_take) = take_from_sender.checked_add(drops) else {
                    return Ter::TEC_INTERNAL;
                };
                take_from_sender = next_take;
            }

            let sender_now = match view.peek(account_keylet(to_160(sender))) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEF_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let sender_balance = sender_sle.get_field_amount(sf("sfBalance")).xrp().drops();
            if sender_balance < take_from_sender {
                return Ter::TEC_FAILED_PROCESSING;
            }
            let mut updated = sender_now.clone_as_object();
            updated.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(sender_balance - take_from_sender)),
            );
            view.update(Arc::new(STLedgerEntry::from_stobject(
                updated,
                *sender_now.key(),
            )))
            .map(|_| Ter::TES_SUCCESS)
            .unwrap_or(Ter::TEF_BAD_LEDGER)
        }
        Asset::Issue(issue) => {
            let mut take_from_sender =
                STAmount::new_with_asset(sf("sfAmount"), Asset::Issue(issue), 0, 0, false);
            for (receiver, amount) in receivers {
                if amount.signum() < 0 {
                    return Ter::TEC_INTERNAL;
                }
                if amount.signum() == 0 || receiver == sender {
                    continue;
                }
                if *sender == issue.account
                    || *receiver == issue.account
                    || issue.account == protocol::no_account()
                {
                    let ter = ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                        view, sender, receiver, amount,
                    );
                    if ter != Ter::TES_SUCCESS {
                        return ter;
                    }
                } else {
                    // Lending explicitly waives the issuer transfer fee.
                    take_from_sender += amount.clone();
                    let ter = ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                        view,
                        &issue.account,
                        receiver,
                        amount,
                    );
                    if ter != Ter::TES_SUCCESS {
                        return ter;
                    }
                }
            }
            if *sender != issue.account && take_from_sender.signum() != 0 {
                return ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                    view,
                    sender,
                    &issue.account,
                    &take_from_sender,
                );
            }
            Ter::TES_SUCCESS
        }
        Asset::MPTIssue(issue) => account_send_multi_mpt(view, sender, issue, receivers),
    }
}

fn direct_send_no_fee_mpt<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    receiver: &AccountID,
    issue: MPTIssue,
    amount: u64,
) -> Ter {
    if amount == 0 || sender == receiver {
        return Ter::TES_SUCCESS;
    }
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let issuance = match view.peek(issuance_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let issuer = issue.issuer();

    if *sender != issuer {
        let sender_token =
            match view.peek(mptoken_keylet_from_mptid(issue.mpt_id(), to_160(sender))) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEC_NO_AUTH,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
        let balance = sender_token.get_field_u64(sf("sfMPTAmount"));
        if balance < amount {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        }
        let mut updated = sender_token.clone_as_object();
        updated.set_field_u64(sf("sfMPTAmount"), balance - amount);
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                updated,
                *sender_token.key(),
            )))
            .is_err()
        {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    let mut issuance_update = issuance.clone_as_object();
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
    if *sender == issuer {
        let Some(next) = outstanding.checked_add(amount) else {
            return Ter::TEC_INTERNAL;
        };
        issuance_update.set_field_u64(sf("sfOutstandingAmount"), next);
    }

    if *receiver == issuer {
        let current = if *sender == issuer {
            issuance_update.get_field_u64(sf("sfOutstandingAmount"))
        } else {
            outstanding
        };
        let Some(next) = current.checked_sub(amount) else {
            return Ter::TEC_INTERNAL;
        };
        issuance_update.set_field_u64(sf("sfOutstandingAmount"), next);
    } else {
        let receiver_token =
            match view.peek(mptoken_keylet_from_mptid(issue.mpt_id(), to_160(receiver))) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEC_NO_AUTH,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
        let balance = receiver_token.get_field_u64(sf("sfMPTAmount"));
        let Some(next) = balance.checked_add(amount) else {
            return Ter::TEC_INTERNAL;
        };
        let mut updated = receiver_token.clone_as_object();
        updated.set_field_u64(sf("sfMPTAmount"), next);
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                updated,
                *receiver_token.key(),
            )))
            .is_err()
        {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    view.update(Arc::new(STLedgerEntry::from_stobject(
        issuance_update,
        *issuance.key(),
    )))
    .map(|_| Ter::TES_SUCCESS)
    .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn account_send_multi_mpt<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    issue: MPTIssue,
    receivers: &[(AccountID, STAmount)],
) -> Ter {
    let issuance = match view.peek(mpt_issuance_keylet_from_mptid(issue.mpt_id())) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let maximum = match u64::try_from(ledger::mptoken_helpers::max_mpt_amount(&issuance)) {
        Ok(value) => value,
        Err(_) => return Ter::TEC_INTERNAL,
    };
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
    let aggregate_fix = view.rules().enabled(&feature_id("fixCleanup3_1_3"));
    let issuer = issue.issuer();
    let mut total_issued = 0_u64;
    let mut take_from_sender = 0_u64;

    for (receiver, amount) in receivers {
        let signed = amount.mpt().value();
        if signed < 0 {
            return Ter::TEC_INTERNAL;
        }
        let amount = signed as u64;
        if amount == 0 || receiver == sender {
            continue;
        }
        if *sender == issuer {
            if ledger::mptoken_helpers::mpt_send_exceeds_maximum_amount(
                amount,
                outstanding,
                maximum,
                total_issued,
                aggregate_fix,
            ) {
                return Ter::TEC_PATH_DRY;
            }
            if aggregate_fix {
                total_issued += amount;
            }
        }

        if *sender == issuer || *receiver == issuer {
            let ter = direct_send_no_fee_mpt(view, sender, receiver, issue, amount);
            if ter != Ter::TES_SUCCESS {
                return ter;
            }
        } else {
            // WaiveTransferFee::Yes: deliver and debit the same exact amount.
            let Some(next_take) = take_from_sender.checked_add(amount) else {
                return Ter::TEC_INTERNAL;
            };
            take_from_sender = next_take;
            let ter = direct_send_no_fee_mpt(view, &issuer, receiver, issue, amount);
            if ter != Ter::TES_SUCCESS {
                return ter;
            }
        }
    }

    if *sender != issuer && take_from_sender != 0 {
        return direct_send_no_fee_mpt(view, sender, &issuer, issue, take_from_sender);
    }
    Ter::TES_SUCCESS
}

pub(super) fn account_send_with_mpt_transfer_waiver<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    waive_mpt_can_transfer: bool,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let from_keylet = account_keylet(to_160(from));
            let to_keylet = account_keylet(to_160(to));
            let from_sle = match view.peek(from_keylet) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEF_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let to_sle = match view.peek(to_keylet) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEF_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let from_balance = from_sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let to_balance = to_sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let drops = amount.xrp().drops();
            if from_balance < drops {
                return Ter::TEC_INSUFFICIENT_FUNDS;
            }
            let mut from_obj = from_sle.clone_as_object();
            from_obj.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(from_balance - drops)),
            );
            let mut to_obj = to_sle.clone_as_object();
            to_obj.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(to_balance + drops)),
            );
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(
                    from_obj,
                    *from_sle.key(),
                )))
                .is_err()
                || view
                    .update(Arc::new(STLedgerEntry::from_stobject(
                        to_obj,
                        *to_sle.key(),
                    )))
                    .is_err()
            {
                return Ter::TEF_BAD_LEDGER;
            }
            Ter::TES_SUCCESS
        }
        Asset::Issue(_) => transfer_iou_no_fee(view, from, to, amount),
        Asset::MPTIssue(issue) => transfer_mpt(
            view,
            sttx,
            issue,
            from,
            to,
            amount.mpt().value().unsigned_abs(),
            waive_mpt_can_transfer,
        ),
    }
}

fn transfer_iou_no_fee<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    if amount.signum() <= 0 || from == to {
        return Ter::TES_SUCCESS;
    }

    let issue = amount.issue();
    if *from == issue.account || *to == issue.account || issue.account.is_zero() {
        return ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, to, amount);
    }

    let res =
        ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, &issue.account, to, amount);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, &issue.account, amount)
}

pub(super) fn asset_issuer(asset: Asset) -> AccountID {
    match asset {
        Asset::Issue(issue) => issue.account,
        Asset::MPTIssue(issue) => issue.issuer(),
    }
}

pub(super) fn token_balance<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    account: &AccountID,
) -> Result<Option<u64>, Ter> {
    view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account)))
        .map(|entry| entry.map(|sle| sle.get_field_u64(sf("sfMPTAmount"))))
        .map_err(|_| Ter::TEF_BAD_LEDGER)
}

pub(super) fn set_token_balance<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    account: &AccountID,
    balance: u64,
) -> Ter {
    let sle = match view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_NO_AUTH,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_u64(sf("sfMPTAmount"), balance);
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub(super) fn transfer_mpt<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    issue: MPTIssue,
    from: &AccountID,
    to: &AccountID,
    amount: u64,
    waive_can_transfer: bool,
) -> Ter {
    if amount == 0 || from == to {
        return Ter::TES_SUCCESS;
    }
    let from_frozen = match ledger::mptoken_helpers::is_frozen_mpt(view, from, &issue) {
        Ok(frozen) => frozen,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let to_frozen = match ledger::mptoken_helpers::is_frozen_mpt(view, to, &issue) {
        Ok(frozen) => frozen,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    if from_frozen || to_frozen {
        return Ter::TEC_LOCKED;
    }
    if !waive_can_transfer {
        match ledger::mptoken_helpers::can_transfer_mpt(view, &issue, from, to) {
            Ok(Ter::TES_SUCCESS) => {}
            Ok(ter) => return ter,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }

    let mpt_id = issue.mpt_id();
    let issuer = issue.issuer();
    if *from != issuer {
        let balance = match token_balance(view, mpt_id, from) {
            Ok(Some(balance)) => balance,
            Ok(None) => return Ter::TEC_NO_AUTH,
            Err(ter) => return ter,
        };
        if balance < amount {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        }
        let ter = set_token_balance(view, mpt_id, from, balance - amount);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    if *to != issuer {
        let prior_balance = match view.peek(account_keylet(to_160(to))) {
            Ok(Some(sle)) => sle.get_field_amount(sf("sfBalance")).xrp(),
            Ok(None) => XRPAmount::new(),
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let ter =
            ledger::add_empty_holding_with_tx(view, sttx, to, prior_balance, &Asset::from(issue));
        if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
            return ter;
        }
        let balance = match token_balance(view, mpt_id, to) {
            Ok(Some(balance)) => balance,
            Ok(None) => return Ter::TEC_NO_AUTH,
            Err(ter) => return ter,
        };
        let Some(next) = balance.checked_add(amount) else {
            return Ter::TEF_INTERNAL;
        };
        let ter = set_token_balance(view, mpt_id, to, next);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    let issuance = match view.peek(mpt_issuance_keylet_from_mptid(mpt_id)) {
        Ok(Some(issuance)) => issuance,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let mut obj = issuance.clone_as_object();
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
    match (*from == issuer, *to == issuer) {
        (true, false) => obj.set_field_u64(sf("sfOutstandingAmount"), outstanding + amount),
        (false, true) => obj.set_field_u64(
            sf("sfOutstandingAmount"),
            outstanding.saturating_sub(amount),
        ),
        _ => {}
    }
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *issuance.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub(super) fn check_cover_sendable<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Ter {
    match asset {
        Asset::Issue(issue) if issue.native() => Ter::TES_SUCCESS,
        Asset::Issue(issue) => {
            match ledger::ripple_state_helpers::try_is_frozen(view, account, &issue) {
                Ok(true) => Ter::TEC_FROZEN,
                Ok(false) => Ter::TES_SUCCESS,
                Err(_) => Ter::TEF_BAD_LEDGER,
            }
        }
        Asset::MPTIssue(issue) => {
            match ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue) {
                Ok(true) => Ter::TEC_LOCKED,
                Ok(false) => Ter::TES_SUCCESS,
                Err(_) => Ter::TEF_BAD_LEDGER,
            }
        }
    }
}

pub(super) fn cover_asset_holding_number<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Result<RuntimeNumber, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() => Ok(view
            .peek(account_keylet(to_160(account)))
            .map_err(|_| Ter::TEF_BAD_LEDGER)?
            .map(|sle| RuntimeNumber::from_i64(sle.get_field_amount(sf("sfBalance")).xrp().drops()))
            .unwrap_or_else(RuntimeNumber::zero)),
        Asset::Issue(issue) if issue.account == *account => {
            Ok(RuntimeNumber::max(get_mantissa_scale()))
        }
        Asset::Issue(issue) => {
            let mut balance = view
                .peek(protocol::line(*account, issue.account, issue.currency))
                .map_err(|_| Ter::TEF_BAD_LEDGER)?
                .map(|line| line.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| {
                    asset
                        .amount(RuntimeNumber::zero())
                        .unwrap_or_else(|_| STAmount::default())
                });
            if *account > issue.account {
                balance.negate();
            }
            Ok(balance.as_number())
        }
        Asset::MPTIssue(issue) if issue.issuer() == *account => {
            Ok(RuntimeNumber::max(get_mantissa_scale()))
        }
        Asset::MPTIssue(issue) => Ok(token_balance(view, issue.mpt_id(), account)?
            .and_then(|balance| i64::try_from(balance).ok())
            .map(RuntimeNumber::from_i64)
            .unwrap_or_else(RuntimeNumber::zero)),
    }
}

pub(super) fn asset_deep_frozen<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Result<bool, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => Ok(false),
        Asset::Issue(issue) => {
            let line = view
                .peek(protocol::line(*account, issue.account, issue.currency))
                .map_err(|_| Ter::TEF_BAD_LEDGER)?;
            let Some(line) = line else {
                return Ok(false);
            };
            Ok(line.is_flag(protocol::lsfLowDeepFreeze)
                || line.is_flag(protocol::lsfHighDeepFreeze))
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue)
            .map_err(|_| Ter::TEF_BAD_LEDGER),
    }
}

pub(super) fn check_asset_deep_frozen<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Ter {
    match asset_deep_frozen(view, account, asset) {
        Ok(false) => return Ter::TES_SUCCESS,
        Ok(true) => {}
        Err(ter) => return ter,
    }
    match asset {
        Asset::MPTIssue(_) => Ter::TEC_LOCKED,
        Asset::Issue(_) => Ter::TEC_FROZEN,
    }
}

pub(super) fn asset_requires_strong_auth<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Result<bool, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => Ok(false),
        Asset::Issue(issue) => {
            let line_keylet = protocol::line(*account, issue.account, issue.currency);
            let trust_line = view.peek(line_keylet).map_err(|_| Ter::TEF_BAD_LEDGER)?;
            let Some(trust_line) = trust_line else {
                return Ok(true);
            };

            let issuer_keylet = protocol::account_keylet(to_160(&issue.account));
            let issuer = view.peek(issuer_keylet).map_err(|_| Ter::TEF_BAD_LEDGER)?;
            if issuer.is_some_and(|issuer| issuer.is_flag(protocol::lsfRequireAuth)) {
                let auth_flag = if *account > issue.account {
                    protocol::lsfLowAuth
                } else {
                    protocol::lsfHighAuth
                };
                return Ok(!trust_line.is_flag(auth_flag));
            }

            Ok(false)
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
            .map(|ter| ter != Ter::TES_SUCCESS)
            .map_err(|_| Ter::TEF_BAD_LEDGER),
    }
}

pub(super) fn check_asset_auth<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
    strong: bool,
) -> Ter {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => Ter::TES_SUCCESS,
        Asset::Issue(issue) => {
            let line_keylet = protocol::line(*account, issue.account, issue.currency);
            let trust_line = match view.peek(line_keylet) {
                Ok(line) => line,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            if trust_line.is_none() && strong {
                return Ter::TEC_NO_LINE;
            }

            let issuer_keylet = protocol::account_keylet(to_160(&issue.account));
            let issuer_requires_auth = match view.peek(issuer_keylet) {
                Ok(issuer) => issuer.is_some_and(|issuer| issuer.is_flag(protocol::lsfRequireAuth)),
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            if issuer_requires_auth {
                let Some(trust_line) = trust_line else {
                    return Ter::TEC_NO_LINE;
                };
                let auth_flag = if *account > issue.account {
                    protocol::lsfLowAuth
                } else {
                    protocol::lsfHighAuth
                };
                if !trust_line.is_flag(auth_flag) {
                    return Ter::TEC_NO_AUTH;
                }
            }

            Ter::TES_SUCCESS
        }
        Asset::MPTIssue(issue) => {
            let auth_type = if strong {
                ledger::mptoken_helpers::MPTAuthType::Strong
            } else {
                ledger::mptoken_helpers::MPTAuthType::Weak
            };
            ledger::mptoken_helpers::require_auth_mpt_with_type(view, &issue, account, auth_type)
                .unwrap_or(Ter::TEF_BAD_LEDGER)
        }
    }
}

pub(super) fn check_mpt_cover_destination_auth<V: ApplyView>(
    view: &mut V,
    destination: &AccountID,
    issue: &MPTIssue,
    require_holding: bool,
) -> Ter {
    if require_holding {
        match view.read(mptoken_keylet_from_mptid(
            issue.mpt_id(),
            to_160(destination),
        )) {
            Ok(Some(_)) => {}
            Ok(None) => return Ter::TEC_NO_AUTH,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }

    let auth_type = if require_holding {
        ledger::mptoken_helpers::MPTAuthType::Strong
    } else {
        ledger::mptoken_helpers::MPTAuthType::Weak
    };
    ledger::mptoken_helpers::require_auth_mpt_with_type(view, issue, destination, auth_type)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub(super) fn check_mpt_cover_transfer<V: ApplyView>(
    view: &mut V,
    source: &AccountID,
    destination: &AccountID,
    owner: &AccountID,
    asset: Asset,
    waive_can_transfer: bool,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if source != &issuer {
        match ledger::mptoken_helpers::is_frozen_mpt(view, source, &issue) {
            Ok(true) => return Ter::TEC_LOCKED,
            Ok(false) => {}
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }
    if destination != &issuer {
        match ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue) {
            Ok(true) => return Ter::TEC_LOCKED,
            Ok(false) => {}
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }

    if !waive_can_transfer {
        let transfer = ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
            .unwrap_or(Ter::TEF_BAD_LEDGER);
        if transfer != Ter::TES_SUCCESS {
            return transfer;
        }
    }

    let auth = check_mpt_cover_destination_auth(view, destination, &issue, destination != owner);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    Ter::TES_SUCCESS
}

pub(super) fn with_asset_number(value: RuntimeNumber, asset: Asset) -> STNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number
}

#[derive(Clone)]
pub(super) struct BrokerCoverState {
    pub(super) key: Uint256,
    pub(super) owner: AccountID,
    pub(super) vault_id: Uint256,
    pub(super) pseudo_account: AccountID,
    pub(super) cover_available: RuntimeNumber,
    pub(super) debt_total: RuntimeNumber,
    pub(super) cover_rate_minimum: u32,
    pub(super) cover_asset: Asset,
}

#[derive(Clone)]
pub(super) struct VaultCoverState {
    pub(super) entry: STLedgerEntry,
    pub(super) asset: Asset,
}

pub(super) fn load_broker<V: ApplyView>(
    view: &mut V,
    broker_id: Uint256,
) -> Result<Option<BrokerCoverState>, Ter> {
    let broker_sle = view
        .peek(protocol::loan_broker_keylet_from_key(broker_id))
        .map_err(|_| Ter::TEF_BAD_LEDGER)?;
    let Some(broker_sle) = broker_sle else {
        return Ok(None);
    };
    Ok(Some(BrokerCoverState {
        key: *broker_sle.key(),
        owner: broker_sle.get_account_id(sf("sfOwner")),
        vault_id: broker_sle.get_field_h256(sf("sfVaultID")),
        pseudo_account: broker_sle.get_account_id(sf("sfAccount")),
        cover_available: broker_sle.get_field_number(sf("sfCoverAvailable")).value(),
        debt_total: if broker_sle.is_field_present(sf("sfDebtTotal")) {
            broker_sle.get_field_number(sf("sfDebtTotal")).value()
        } else {
            RuntimeNumber::zero()
        },
        cover_rate_minimum: if broker_sle.is_field_present(sf("sfCoverRateMinimum")) {
            broker_sle.get_field_u32(sf("sfCoverRateMinimum"))
        } else {
            0
        },
        cover_asset: broker_sle.get_field_issue(sf("sfAsset")).asset(),
    }))
}

pub(super) fn load_vault<V: ApplyView>(
    view: &mut V,
    vault_id: Uint256,
) -> Result<Option<VaultCoverState>, Ter> {
    let vault_sle = view
        .peek(protocol::vault_keylet_from_key(vault_id))
        .map_err(|_| Ter::TEF_BAD_LEDGER)?;
    let Some(vault_sle) = vault_sle else {
        return Ok(None);
    };
    Ok(Some(VaultCoverState {
        entry: (*vault_sle).clone(),
        asset: vault_sle.get_field_issue(sf("sfAsset")).asset(),
    }))
}

pub(super) fn persist_broker_cover<V: ApplyView>(
    view: &mut V,
    broker_id: Uint256,
    broker: &BrokerCoverState,
) -> Ter {
    let sle = match view.peek(protocol::loan_broker_keylet_from_key(broker_id)) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_number(
        sf("sfCoverAvailable"),
        with_asset_number(broker.cover_available, broker.cover_asset),
    );
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, broker.key)))
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

pub(super) fn runtime_number_floor_to_u32(value: RuntimeNumber) -> u32 {
    if value <= RuntimeNumber::zero() {
        return 0;
    }

    let Ok((mantissa, exponent)) = value.external_parts() else {
        return u32::MAX;
    };
    if mantissa <= 0 {
        return 0;
    }

    let mut magnitude = mantissa as u128;
    if exponent >= 0 {
        for _ in 0..exponent {
            magnitude = magnitude.saturating_mul(10);
            if magnitude > u128::from(u32::MAX) {
                return u32::MAX;
            }
        }
    } else {
        for _ in 0..(-exponent) {
            magnitude /= 10;
            if magnitude == 0 {
                return 0;
            }
        }
    }

    u32::try_from(magnitude).unwrap_or(u32::MAX)
}

pub(super) fn runtime_number_ceil_to_u64(value: RuntimeNumber) -> u64 {
    if value <= RuntimeNumber::zero() {
        return 0;
    }

    let Ok((mantissa, exponent)) = value.external_parts() else {
        return u64::MAX;
    };
    if mantissa <= 0 {
        return 0;
    }

    let mut magnitude = mantissa as u128;
    let mut remainder = false;
    if exponent >= 0 {
        for _ in 0..exponent {
            magnitude = magnitude.saturating_mul(10);
            if magnitude > u128::from(u64::MAX) {
                return u64::MAX;
            }
        }
    } else {
        for _ in 0..(-exponent) {
            remainder |= !magnitude.is_multiple_of(10);
            magnitude /= 10;
        }
    }

    if remainder {
        magnitude = magnitude.saturating_add(1);
    }
    u64::try_from(magnitude).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod storage_failure_tests {
    use super::*;
    use basics::base_uint::Uint256;
    use ledger::{ApplyViewImpl, Ledger, RawView, ReadView, ReadViewTx, ViewError};
    use protocol::{ApplyFlags, LedgerEntryType, MPTAmount, Rules};

    fn raw(account: AccountID) -> basics::base_uint::Uint160 {
        basics::base_uint::Uint160::from_void(account.data())
    }

    fn mpt_amount(issue: MPTIssue, value: u64) -> STAmount {
        STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(value as i64), issue)
    }

    fn issuance_entry(issue: MPTIssue, outstanding: u64, maximum: u64) -> STLedgerEntry {
        let keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
        let mut sle =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, keylet.key);
        sle.set_account_id(sf("sfIssuer"), issue.issuer());
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u64(sf("sfOutstandingAmount"), outstanding);
        sle.set_field_u64(sf("sfMaximumAmount"), maximum);
        sle.set_field_u32(sf("sfFlags"), 0);
        sle.set_field_u64(sf("sfOwnerNode"), 0);
        sle
    }

    fn token_entry(issue: MPTIssue, holder: AccountID, amount: u64) -> STLedgerEntry {
        let keylet = mptoken_keylet_from_mptid(issue.mpt_id(), raw(holder));
        let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::MPToken, keylet.key);
        sle.set_account_id(sf("sfAccount"), holder);
        sle.set_field_h192(sf("sfMPTokenIssuanceID"), issue.mpt_id());
        sle.set_field_u64(sf("sfMPTAmount"), amount);
        sle.set_field_u32(sf("sfFlags"), 0);
        sle.set_field_u64(sf("sfOwnerNode"), 0);
        sle
    }

    fn account_entry(account: AccountID, drops: i64) -> STLedgerEntry {
        let keylet = account_keylet(raw(account));
        let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
        sle.set_account_id(sf("sfAccount"), account);
        sle.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(drops)),
        );
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u32(sf("sfOwnerCount"), 0);
        sle.set_field_u32(sf("sfFlags"), 0);
        sle
    }

    fn mpt_view(entries: Vec<STLedgerEntry>) -> ApplyViewImpl<Ledger> {
        let mut ledger = Ledger::from_ledger_seq_and_close_time(1, 0, false);
        ledger.set_rules(Rules::new([feature_id("fixCleanup3_1_3")]));
        for entry in entries {
            ledger.raw_insert(Arc::new(entry)).expect("seed MPT entry");
        }
        ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE)
    }

    #[derive(Debug)]
    struct FaultReadView {
        base: Arc<Ledger>,
    }

    impl ReadView for FaultReadView {
        fn open(&self) -> bool {
            ReadView::open(self.base.as_ref())
        }
        fn header(&self) -> ledger::LedgerHeader {
            ReadView::header(self.base.as_ref())
        }
        fn fees(&self) -> ledger::Fees {
            ReadView::fees(self.base.as_ref())
        }
        fn rules(&self) -> protocol::Rules {
            ReadView::rules(self.base.as_ref())
        }
        fn exists(&self, key: protocol::Keylet) -> Result<bool, ViewError> {
            ReadView::exists(self.base.as_ref(), key)
        }
        fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            ReadView::succ(self.base.as_ref(), key, last)
        }
        fn read(&self, _key: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion(
                "injected lending read failure".into(),
            ))
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            ReadView::sles(self.base.as_ref())
        }
        fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
            ReadView::tx_exists(self.base.as_ref(), key)
        }
        fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            ReadView::tx_read(self.base.as_ref(), key)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            ReadView::txs(self.base.as_ref())
        }
    }

    #[test]
    fn lending_balance_and_token_reads_fail_closed() {
        let faulty = Arc::new(FaultReadView {
            base: Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
        });
        let mut view = ApplyViewImpl::new(faulty, ApplyFlags::NONE);
        let account = AccountID::from_array([0x55; 20]);
        assert_eq!(
            cover_asset_holding_number(&mut view, &account, Asset::Issue(protocol::xrp_issue())),
            Err(Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(
            token_balance(&mut view, Uint192::from_array([0x11; 24]), &account),
            Err(Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(
            super::super::helpers::is_pseudo_account(&mut view, &account),
            Err(Ter::TEF_BAD_LEDGER)
        );

        let issuer = AccountID::from_array([0x56; 20]);
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), issuer);
        let amount = STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(1, 0).expect("canonical IOU amount"),
            issue,
        );
        assert_eq!(
            account_send_multi(
                &mut view,
                &issuer,
                Asset::Issue(issue),
                &[
                    (AccountID::from_array([0x57; 20]), amount.clone()),
                    (AccountID::from_array([0x58; 20]), amount),
                ],
            ),
            Ter::TEF_BAD_LEDGER
        );
    }

    #[test]
    fn account_send_multi_mpt_checks_the_aggregate_issuer_cap() {
        let issuer = AccountID::from_array([0x61; 20]);
        let first = AccountID::from_array([0x62; 20]);
        let second = AccountID::from_array([0x63; 20]);
        let issue = MPTIssue::new(protocol::make_mpt_id(1, issuer));
        let mut view = mpt_view(vec![
            issuance_entry(issue, 100, 150),
            token_entry(issue, first, 0),
            token_entry(issue, second, 0),
        ]);

        assert_eq!(
            account_send_multi(
                &mut view,
                &issuer,
                Asset::MPTIssue(issue),
                &[
                    (first, mpt_amount(issue, 30)),
                    (second, mpt_amount(issue, 30))
                ],
            ),
            Ter::TEC_PATH_DRY
        );
        // Pinned directSendNoLimitMultiMPT discovers the aggregate overflow
        // on the second receiver. The outer transaction sandbox rolls this
        // staged prefix back after the tec result.
        assert_eq!(
            view.peek(mptoken_keylet_from_mptid(issue.mpt_id(), raw(first)))
                .expect("read first token")
                .expect("first token exists")
                .get_field_u64(sf("sfMPTAmount")),
            30
        );
        assert_eq!(
            view.peek(mptoken_keylet_from_mptid(issue.mpt_id(), raw(second)))
                .expect("read second token")
                .expect("second token exists")
                .get_field_u64(sf("sfMPTAmount")),
            0
        );
    }

    #[test]
    fn account_send_multi_mpt_debits_third_party_once_for_all_receivers() {
        let issuer = AccountID::from_array([0x71; 20]);
        let sender = AccountID::from_array([0x72; 20]);
        let first = AccountID::from_array([0x73; 20]);
        let second = AccountID::from_array([0x74; 20]);
        let issue = MPTIssue::new(protocol::make_mpt_id(1, issuer));
        let mut view = mpt_view(vec![
            issuance_entry(issue, 1_000, 10_000),
            token_entry(issue, sender, 100),
            token_entry(issue, first, 0),
            token_entry(issue, second, 0),
        ]);

        assert_eq!(
            account_send_multi(
                &mut view,
                &sender,
                Asset::MPTIssue(issue),
                &[
                    (first, mpt_amount(issue, 30)),
                    (second, mpt_amount(issue, 20))
                ],
            ),
            Ter::TES_SUCCESS
        );
        let balance = |view: &mut ApplyViewImpl<Ledger>, account| {
            view.peek(mptoken_keylet_from_mptid(issue.mpt_id(), raw(account)))
                .expect("read token")
                .expect("token exists")
                .get_field_u64(sf("sfMPTAmount"))
        };
        assert_eq!(balance(&mut view, sender), 50);
        assert_eq!(balance(&mut view, first), 30);
        assert_eq!(balance(&mut view, second), 20);
        assert_eq!(
            view.peek(mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .expect("read issuance")
                .expect("issuance exists")
                .get_field_u64(sf("sfOutstandingAmount")),
            1_000
        );
    }

    #[test]
    fn account_send_multi_xrp_preserves_pinned_receiver_before_sender_failure_order() {
        let sender = AccountID::from_array([0x81; 20]);
        let first = AccountID::from_array([0x82; 20]);
        let second = AccountID::from_array([0x83; 20]);
        let mut view = mpt_view(vec![
            account_entry(sender, 50),
            account_entry(first, 10),
            account_entry(second, 20),
        ]);

        assert_eq!(
            account_send_multi(
                &mut view,
                &sender,
                Asset::Issue(protocol::xrp_issue()),
                &[
                    (first, STAmount::from_xrp_amount(XRPAmount::from_drops(30))),
                    (second, STAmount::from_xrp_amount(XRPAmount::from_drops(25))),
                ],
            ),
            Ter::TEC_FAILED_PROCESSING
        );
        let xrp = |view: &mut ApplyViewImpl<Ledger>, account| {
            view.peek(account_keylet(raw(account)))
                .expect("read account")
                .expect("account exists")
                .get_field_amount(sf("sfBalance"))
                .xrp()
                .drops()
        };
        assert_eq!(xrp(&mut view, sender), 50);
        assert_eq!(xrp(&mut view, first), 40);
        assert_eq!(xrp(&mut view, second), 45);
    }
}
