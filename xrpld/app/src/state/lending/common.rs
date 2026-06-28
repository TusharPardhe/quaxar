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
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    account_send_with_mpt_transfer_waiver(view, from, to, amount, false)
}

pub(super) fn account_send_with_mpt_transfer_waiver<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    waive_mpt_can_transfer: bool,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let from_keylet = account_keylet(to_160(from));
            let to_keylet = account_keylet(to_160(to));
            let Ok(Some(from_sle)) = view.peek(from_keylet) else {
                return Ter::TEF_BAD_LEDGER;
            };
            let Ok(Some(to_sle)) = view.peek(to_keylet) else {
                return Ter::TEF_BAD_LEDGER;
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
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                from_obj,
                *from_sle.key(),
            )));
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                to_obj,
                *to_sle.key(),
            )));
            Ter::TES_SUCCESS
        }
        Asset::Issue(_) => transfer_iou_no_fee(view, from, to, amount),
        Asset::MPTIssue(issue) => transfer_mpt(
            view,
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
) -> Option<u64> {
    view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u64(sf("sfMPTAmount")))
}

pub(super) fn set_token_balance<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    account: &AccountID,
    balance: u64,
) -> Ter {
    let Ok(Some(sle)) = view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_u64(sf("sfMPTAmount"), balance);
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub(super) fn transfer_mpt<V: ApplyView>(
    view: &mut V,
    issue: MPTIssue,
    from: &AccountID,
    to: &AccountID,
    amount: u64,
    waive_can_transfer: bool,
) -> Ter {
    if amount == 0 || from == to {
        return Ter::TES_SUCCESS;
    }
    if ledger::mptoken_helpers::is_frozen_mpt(view, from, &issue).unwrap_or(false)
        || ledger::mptoken_helpers::is_frozen_mpt(view, to, &issue).unwrap_or(false)
    {
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
        let Some(balance) = token_balance(view, mpt_id, from) else {
            return Ter::TEF_BAD_LEDGER;
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
        let prior_balance = view
            .peek(account_keylet(to_160(to)))
            .ok()
            .flatten()
            .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp())
            .unwrap_or_default();
        let ter = ledger::add_empty_holding(view, to, prior_balance, &Asset::from(issue));
        if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
            return ter;
        }
        let Some(balance) = token_balance(view, mpt_id, to) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Some(next) = balance.checked_add(amount) else {
            return Ter::TEF_INTERNAL;
        };
        let ter = set_token_balance(view, mpt_id, to, next);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    let Ok(Some(issuance)) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id)) else {
        return Ter::TEF_BAD_LEDGER;
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
            if ledger::ripple_state_helpers::is_frozen(view, account, &issue) {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            }
        }
        Asset::MPTIssue(issue) => {
            if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(false) {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            }
        }
    }
}

pub(super) fn cover_asset_holding_number<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> RuntimeNumber {
    match asset {
        Asset::Issue(issue) if issue.native() => view
            .peek(account_keylet(to_160(account)))
            .ok()
            .flatten()
            .map(|sle| RuntimeNumber::from_i64(sle.get_field_amount(sf("sfBalance")).xrp().drops()))
            .unwrap_or_else(RuntimeNumber::zero),
        Asset::Issue(issue) if issue.account == *account => {
            RuntimeNumber::max(get_mantissa_scale())
        }
        Asset::Issue(issue) => {
            let mut balance = view
                .peek(protocol::line(*account, issue.account, issue.currency))
                .ok()
                .flatten()
                .map(|line| line.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| {
                    asset
                        .amount(RuntimeNumber::zero())
                        .unwrap_or_else(|_| STAmount::default())
                });
            if *account > issue.account {
                balance.negate();
            }
            balance.as_number()
        }
        Asset::MPTIssue(issue) if issue.issuer() == *account => {
            RuntimeNumber::max(get_mantissa_scale())
        }
        Asset::MPTIssue(issue) => token_balance(view, issue.mpt_id(), account)
            .and_then(|balance| i64::try_from(balance).ok())
            .map(RuntimeNumber::from_i64)
            .unwrap_or_else(RuntimeNumber::zero),
    }
}

pub(super) fn asset_deep_frozen<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => false,
        Asset::Issue(issue) => {
            let Some(line) = view
                .peek(protocol::line(*account, issue.account, issue.currency))
                .ok()
                .flatten()
                .or_else(|| {
                    view.read(protocol::line(*account, issue.account, issue.currency))
                        .ok()
                        .flatten()
                })
            else {
                return false;
            };
            line.is_flag(protocol::lsfLowDeepFreeze) || line.is_flag(protocol::lsfHighDeepFreeze)
        }
        Asset::MPTIssue(issue) => {
            ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true)
        }
    }
}

pub(super) fn check_asset_deep_frozen<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> Ter {
    if !asset_deep_frozen(view, account, asset) {
        return Ter::TES_SUCCESS;
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
) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => false,
        Asset::Issue(issue) => {
            let line_keylet = protocol::line(*account, issue.account, issue.currency);
            let trust_line = view
                .peek(line_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(line_keylet).ok().flatten());
            let Some(trust_line) = trust_line else {
                return true;
            };

            let issuer_keylet = protocol::account_keylet(to_160(&issue.account));
            if let Some(issuer) = view
                .peek(issuer_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(issuer_keylet).ok().flatten())
                && issuer.is_flag(protocol::lsfRequireAuth)
            {
                let auth_flag = if *account > issue.account {
                    protocol::lsfLowAuth
                } else {
                    protocol::lsfHighAuth
                };
                return !trust_line.is_flag(auth_flag);
            }

            false
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
            .map(|ter| ter != Ter::TES_SUCCESS)
            .unwrap_or(true),
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
            let trust_line = view
                .peek(line_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(line_keylet).ok().flatten());
            if trust_line.is_none() && strong {
                return Ter::TEC_NO_LINE;
            }

            let issuer_keylet = protocol::account_keylet(to_160(&issue.account));
            let issuer_requires_auth = view
                .peek(issuer_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(issuer_keylet).ok().flatten())
                .is_some_and(|issuer| issuer.is_flag(protocol::lsfRequireAuth));
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
    if require_holding
        && view
            .read(mptoken_keylet_from_mptid(
                issue.mpt_id(),
                to_160(destination),
            ))
            .ok()
            .flatten()
            .is_none()
    {
        return Ter::TEC_NO_AUTH;
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

    if source != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, source, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
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
) -> Option<BrokerCoverState> {
    let broker_sle = view
        .peek(protocol::loan_broker_keylet_from_key(broker_id))
        .ok()
        .flatten()?;
    Some(BrokerCoverState {
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
    })
}

pub(super) fn load_vault<V: ApplyView>(view: &mut V, vault_id: Uint256) -> Option<VaultCoverState> {
    let vault_sle = view
        .peek(protocol::vault_keylet_from_key(vault_id))
        .ok()
        .flatten()?;
    Some(VaultCoverState {
        entry: (*vault_sle).clone(),
        asset: vault_sle.get_field_issue(sf("sfAsset")).asset(),
    })
}

pub(super) fn persist_broker_cover<V: ApplyView>(
    view: &mut V,
    broker_id: Uint256,
    broker: &BrokerCoverState,
) -> Ter {
    let Ok(Some(sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_number(
        sf("sfCoverAvailable"),
        with_asset_number(broker.cover_available, broker.cover_asset),
    );
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, broker.key)));
    Ter::TES_SUCCESS
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
