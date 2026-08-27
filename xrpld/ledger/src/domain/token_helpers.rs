//! the reference implementation parity helpers for balance lookup plus the narrow
//! holding lifecycle used by vault and lending transactors.

use std::sync::Arc;

use crate::{ApplyView, Ledger, ReadView, is_deep_frozen, is_frozen};
use basics::base_uint::Uint160;
use protocol::{
    AccountID, Asset, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTIssue, STAmount,
    STLedgerEntry, STObject, STTx, StBase, XRPAmount, account_keylet, feature_id,
    get_field_by_symbol, line, lsfDefaultRipple, lsfMPTLocked, owner_dir_keylet, sf_generic,
};
use shamap::traversal::TraversalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeHandling {
    IgnoreFreeze,
    ZeroIfFrozen,
}

fn zero_iou(issue: Issue) -> STAmount {
    STAmount::from_iou_amount(sf_generic(), IOUAmount::new(), issue)
}

fn zero_mpt(issue: MPTIssue) -> STAmount {
    STAmount::from_mpt_amount(sf_generic(), MPTAmount::new(), issue)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn effective_tx_reserve_sponsor<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    account_sle: &Arc<STLedgerEntry>,
) -> Result<Option<Arc<STLedgerEntry>>, protocol::Ter> {
    if !view.rules().enabled(&feature_id("Sponsor"))
        || crate::is_pseudo_account(account_sle)
        || account_sle.get_account_id(sf("sfAccount")) != tx.get_account_id(sf("sfAccount"))
        || !tx.is_field_present(sf("sfSponsor"))
        || !tx.is_field_present(sf("sfSponsorFlags"))
        || !crate::is_reserve_sponsored(tx.get_field_u32(sf("sfSponsorFlags")))
    {
        return Ok(None);
    }

    let sponsor = tx.get_account_id(sf("sfSponsor"));
    view.peek(account_keylet(to_uint160(sponsor)))
        .map_err(|_| protocol::Ter::TEF_BAD_LEDGER)?
        .ok_or(protocol::Ter::TEC_INTERNAL)
        .map(Some)
}

fn check_holding_reserve<V: ApplyView>(
    view: &V,
    tx: Option<&STTx>,
    account_sle: &Arc<STLedgerEntry>,
    prior_balance: XRPAmount,
    sponsor: Option<&Arc<STLedgerEntry>>,
    insufficient: protocol::Ter,
) -> Result<(), protocol::Ter> {
    if let Some(sponsor_sle) = sponsor {
        let sponsor_id = sponsor_sle.get_account_id(sf("sfAccount"));
        let account_id = account_sle.get_account_id(sf("sfAccount"));
        let sponsorship = view
            .read(protocol::sponsorship_keylet(
                to_uint160(sponsor_id),
                to_uint160(account_id),
            ))
            .map_err(|_| protocol::Ter::TEF_BAD_LEDGER)?;
        let sponsor_signed = tx.is_some_and(|tx| tx.is_field_present(sf("sfSponsorSignature")));
        if sponsorship.is_none() && !sponsor_signed {
            return Err(protocol::Ter::TEC_INTERNAL);
        }
        if sponsorship
            .as_ref()
            .is_some_and(|sle| sle.get_field_u32(sf("sfRemainingOwnerCount")) < 1)
        {
            return Err(insufficient);
        }
        let reserve = crate::effective_account_reserve(view.fees(), sponsor_sle, 1, 0);
        if sponsor_sle.get_field_amount(sf("sfBalance")).xrp().drops() < reserve as i64 {
            return Err(insufficient);
        }
    } else {
        let reserve = crate::effective_account_reserve(view.fees(), account_sle, 1, 0);
        if prior_balance.drops() < reserve as i64 {
            return Err(insufficient);
        }
    }
    Ok(())
}

pub fn xrp_liquid(
    ledger: &Ledger,
    account: AccountID,
    owner_count_adj: i32,
) -> Result<protocol::XRPAmount, TraversalError> {
    let Some(account_root) = ledger.read(account_keylet(to_uint160(account)))? else {
        return Ok(protocol::XRPAmount::new());
    };

    let reserve = if crate::is_pseudo_account(&account_root) {
        0
    } else {
        crate::effective_account_reserve(ledger.fees(), &account_root, owner_count_adj, 0)
    };
    let balance = account_root
        .get_field_amount(protocol::get_field_by_symbol("sfBalance"))
        .xrp();
    // Fees are sourced from ledger state. Do not let a malformed/out-of-range
    // reserve wrap through `as i64` or panic while evaluating liquidity.
    // Fee settings are ledger state; an unrepresentable XRPAmount is malformed
    // state and must propagate through the existing traversal error channel.
    let Ok(reserve_drops) = i64::try_from(reserve) else {
        return Err(TraversalError::View);
    };

    Ok(if balance.drops() < reserve_drops {
        protocol::XRPAmount::new()
    } else {
        balance - protocol::XRPAmount::from_drops(reserve_drops)
    })
}

pub fn account_funds(
    ledger: &Ledger,
    account: AccountID,
    default_amount: &STAmount,
    freeze_handling: FreezeHandling,
) -> Result<STAmount, TraversalError> {
    let Asset::Issue(issue) = default_amount.asset() else {
        let Asset::MPTIssue(issue) = default_amount.asset() else {
            unreachable!("all assets are handled");
        };

        if issue.issuer() == account {
            let Some(issuance) =
                ledger.read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))?
            else {
                return Ok(zero_mpt(issue));
            };
            return Ok(STAmount::from_mpt_amount(
                sf_generic(),
                MPTAmount::from_value(crate::mptoken_helpers::available_mpt_amount(&issuance)),
                issue,
            ));
        }

        let token_key = protocol::mptoken_keylet_from_mptid(issue.mpt_id(), to_uint160(account));
        if freeze_handling == FreezeHandling::ZeroIfFrozen
            && ledger
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))?
                .as_ref()
                .is_some_and(|issuance| issuance.is_flag(lsfMPTLocked))
        {
            return Ok(zero_mpt(issue));
        }

        let Some(token) = ledger.read(token_key)? else {
            return Ok(zero_mpt(issue));
        };
        if freeze_handling == FreezeHandling::ZeroIfFrozen && token.is_flag(lsfMPTLocked) {
            return Ok(zero_mpt(issue));
        }

        return Ok(STAmount::from_mpt_amount(
            sf_generic(),
            MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
            issue,
        ));
    };

    if issue.native() {
        return Ok(STAmount::from_xrp_amount(xrp_liquid(ledger, account, 0)?));
    }

    if issue.issuer() == account {
        return Ok(default_amount.clone());
    }

    if freeze_handling == FreezeHandling::ZeroIfFrozen
        && (is_frozen(
            ledger,
            to_uint160(account),
            issue.currency,
            to_uint160(issue.issuer()),
        )? || is_deep_frozen(
            ledger,
            to_uint160(account),
            issue.currency,
            to_uint160(issue.issuer()),
        )?)
    {
        return Ok(zero_iou(issue));
    }

    let mut amount = zero_iou(issue);
    if let Some(trustline) = ledger.read(line(account, issue.issuer(), issue.currency))? {
        amount = trustline.get_field_amount(get_field_by_symbol("sfBalance"));
        if account > issue.issuer() {
            amount.negate();
        }
        amount.set_issuer(issue.issuer());
    }
    Ok(amount)
}

pub fn account_funds_text(
    ledger: &Ledger,
    account: AccountID,
    default_amount: &STAmount,
    freeze_handling: FreezeHandling,
) -> Result<String, TraversalError> {
    Ok(account_funds(ledger, account, default_amount, freeze_handling)?.text())
}

pub fn can_add_holding<V: ReadView>(view: &V, asset: &Asset) -> protocol::Ter {
    asset.visit(
        |issue| {
            if issue.native() {
                return protocol::Ter::TES_SUCCESS;
            }
            let issuer = match view.read(account_keylet(to_uint160(issue.issuer()))) {
                Ok(Some(issuer)) => issuer,
                Ok(None) => return protocol::Ter::TER_NO_ACCOUNT,
                Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
            };
            if issuer.get_field_u32(sf("sfFlags")) & lsfDefaultRipple == 0 {
                return protocol::Ter::TER_NO_RIPPLE;
            }
            protocol::Ter::TES_SUCCESS
        },
        |mpt_issue| {
            let issuance =
                match view.read(protocol::mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id())) {
                    Ok(Some(issuance)) => issuance,
                    Ok(None) => return protocol::Ter::TEC_OBJECT_NOT_FOUND,
                    Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
                };
            if issuance.get_field_u32(sf("sfFlags")) & protocol::lsfMPTCanTransfer == 0 {
                return protocol::Ter::TEC_NO_AUTH;
            }
            protocol::Ter::TES_SUCCESS
        },
    )
}

/// `ApplyViewContext`-equivalent entry point for transaction application.
/// Reserve sponsorship only applies when `account` is the transaction's own
/// non-pseudo AccountRoot, exactly as rippled's
/// `getEffectiveTxReserveSponsor(ctx, accountSle)` requires.
pub fn add_empty_holding_with_tx<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    account: &AccountID,
    prior_balance: XRPAmount,
    asset: &Asset,
) -> protocol::Ter {
    add_empty_holding_with_context(view, tx, account, prior_balance, asset)
}

fn add_empty_holding_with_context<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    account: &AccountID,
    prior_balance: XRPAmount,
    asset: &Asset,
) -> protocol::Ter {
    match asset {
        Asset::Issue(issue) => add_empty_iou_holding(view, tx, account, prior_balance, issue),
        Asset::MPTIssue(mpt_issue) => {
            add_empty_mpt_holding(view, tx, account, prior_balance, mpt_issue)
        }
    }
}

pub fn remove_empty_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: &Asset,
) -> protocol::Ter {
    match asset {
        Asset::Issue(issue) => remove_empty_iou_holding(view, account, issue),
        Asset::MPTIssue(mpt_issue) => remove_empty_mpt_holding(view, account, mpt_issue),
    }
}

fn erase_empty_owner_dir_root<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
) -> Result<(), protocol::Ter> {
    let keylet = owner_dir_keylet(to_uint160(*account));
    crate::empty_dir_delete(view, &keylet).map_err(|_| protocol::Ter::TEF_BAD_LEDGER)?;
    Ok(())
}

fn add_empty_iou_holding<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    account: &AccountID,
    prior_balance: XRPAmount,
    issue: &Issue,
) -> protocol::Ter {
    if issue.native() || *account == issue.issuer() {
        return protocol::Ter::TES_SUCCESS;
    }
    let src = issue.issuer();
    let dst = *account;
    let high = src > dst;
    let line_keylet = line(src, dst, issue.currency);
    let src_sle = match view.peek(account_keylet(to_uint160(src))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return protocol::Ter::TEF_INTERNAL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    let dst_sle = match view.peek(account_keylet(to_uint160(dst))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return protocol::Ter::TEF_INTERNAL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    let sponsor = match effective_tx_reserve_sponsor(view, tx, &dst_sle) {
        Ok(sponsor) => sponsor,
        Err(ter) => return ter,
    };
    if src_sle.get_field_u32(sf("sfFlags")) & protocol::lsfGlobalFreeze != 0 {
        return protocol::Ter::TEC_FROZEN;
    }
    if src_sle.get_field_u32(sf("sfFlags")) & lsfDefaultRipple == 0 {
        return protocol::Ter::TEC_INTERNAL;
    }
    match view.read(line_keylet) {
        Ok(Some(_)) => return protocol::Ter::TEC_DUPLICATE,
        Ok(None) => {}
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    }

    if let Err(ter) = check_holding_reserve(
        view,
        Some(tx),
        &dst_sle,
        prior_balance,
        sponsor.as_ref(),
        protocol::Ter::TEC_NO_LINE_INSUF_RESERVE,
    ) {
        return ter;
    }

    let mut obj = STObject::new(sf_generic());
    obj.set_field_u16(sf("sfLedgerEntryType"), LedgerEntryType::RippleState as u16);
    obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf("sfBalance"),
            IOUAmount::new(),
            Issue::new(issue.currency, protocol::no_account()),
        ),
    );
    let low_limit = STAmount::new_with_asset(
        sf("sfLowLimit"),
        Asset::Issue(Issue::new(issue.currency, if high { dst } else { src })),
        0,
        0,
        false,
    );
    let high_limit = STAmount::new_with_asset(
        sf("sfHighLimit"),
        Asset::Issue(Issue::new(issue.currency, if high { src } else { dst })),
        0,
        0,
        false,
    );
    obj.set_field_amount(sf("sfLowLimit"), low_limit);
    obj.set_field_amount(sf("sfHighLimit"), high_limit);
    // `high` means the issuer/source sorts high, so the receiving account
    // sorts low (and vice versa). Reserve ownership follows the receiver.
    let mut flags = if high {
        protocol::lsfLowReserve
    } else {
        protocol::lsfHighReserve
    };
    flags |= if high { 0x0010_0000 } else { 0x0020_0000 };
    obj.set_field_u32(sf("sfFlags"), flags);

    let low_account = if high { dst } else { src };
    let low_dir = owner_dir_keylet(to_uint160(low_account));
    let low_node = match crate::dir_insert(
        view,
        &low_dir,
        line_keylet.key,
        &crate::describe_owner_dir(low_account),
    ) {
        Ok(Some(page)) => page,
        Ok(None) => return protocol::Ter::TEC_DIR_FULL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    let high_account = if high { src } else { dst };
    let high_dir = owner_dir_keylet(to_uint160(high_account));
    let high_node = match crate::dir_insert(
        view,
        &high_dir,
        line_keylet.key,
        &crate::describe_owner_dir(high_account),
    ) {
        Ok(Some(page)) => page,
        Ok(None) => return protocol::Ter::TEC_DIR_FULL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    obj.set_field_u64(sf("sfLowNode"), low_node);
    obj.set_field_u64(sf("sfHighNode"), high_node);
    if let Some(sponsor_sle) = sponsor.as_ref() {
        obj.set_account_id(
            if high {
                sf("sfLowSponsor")
            } else {
                sf("sfHighSponsor")
            },
            sponsor_sle.get_account_id(sf("sfAccount")),
        );
    }

    let line_sle = Arc::new(STLedgerEntry::from_stobject(obj, line_keylet.key));
    if view.insert(Arc::clone(&line_sle)).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if crate::increase_owner_count_for_object(view, &dst_sle, sponsor.as_ref()).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    protocol::Ter::TES_SUCCESS
}

fn remove_empty_iou_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issue: &Issue,
) -> protocol::Ter {
    if issue.native() {
        let sle = match view.peek(account_keylet(to_uint160(*account))) {
            Ok(Some(sle)) => sle,
            Ok(None) => return protocol::Ter::TEC_INTERNAL,
            Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
        };
        if sle.get_field_amount(sf("sfBalance")).xrp().drops() != 0 {
            return protocol::Ter::TEC_HAS_OBLIGATIONS;
        }
        return protocol::Ter::TES_SUCCESS;
    }

    let account_is_issuer = *account == issue.issuer();
    let line_keylet = line(*account, issue.issuer(), issue.currency);
    let line_sle = match view.peek(line_keylet) {
        Ok(Some(line_sle)) => line_sle,
        Ok(None) => {
            return if account_is_issuer {
                protocol::Ter::TES_SUCCESS
            } else {
                protocol::Ter::TEC_OBJECT_NOT_FOUND
            };
        }
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    if !account_is_issuer && line_sle.get_field_amount(sf("sfBalance")).signum() != 0 {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }

    let low_limit = line_sle.get_field_amount(sf("sfLowLimit")).issue().issuer();
    let high_limit = line_sle
        .get_field_amount(sf("sfHighLimit"))
        .issue()
        .issuer();
    let mut deleted = line_sle.clone_as_object();
    for (reserve_flag, sponsor_field, owner) in [
        (protocol::lsfLowReserve, sf("sfLowSponsor"), low_limit),
        (protocol::lsfHighReserve, sf("sfHighSponsor"), high_limit),
    ] {
        if line_sle.get_field_u32(sf("sfFlags")) & reserve_flag == 0 {
            continue;
        }
        let owner_sle = match view.peek(account_keylet(to_uint160(owner))) {
            Ok(Some(sle)) => sle,
            Ok(None) => return protocol::Ter::TEC_INTERNAL,
            Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
        };
        let sponsor = line_sle
            .is_field_present(sponsor_field)
            .then(|| line_sle.get_account_id(sponsor_field));
        if crate::decrease_owner_count_for_trust_line(view, &owner_sle, sponsor).is_err() {
            return protocol::Ter::TEF_BAD_LEDGER;
        }
        deleted.set_field_u32(
            sf("sfFlags"),
            deleted.get_field_u32(sf("sfFlags")) & !reserve_flag,
        );
        deleted.make_field_absent(sponsor_field);
    }
    let line_sle = Arc::new(STLedgerEntry::from_stobject(deleted, *line_sle.key()));

    if !matches!(
        crate::dir_remove(
            view,
            &owner_dir_keylet(to_uint160(low_limit)),
            line_sle.get_field_u64(sf("sfLowNode")),
            *line_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if !matches!(
        crate::dir_remove(
            view,
            &owner_dir_keylet(to_uint160(high_limit)),
            line_sle.get_field_u64(sf("sfHighNode")),
            *line_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if view.erase(line_sle).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if erase_empty_owner_dir_root(view, &low_limit).is_err()
        || erase_empty_owner_dir_root(view, &high_limit).is_err()
    {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    protocol::Ter::TES_SUCCESS
}

fn add_empty_mpt_holding<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    account: &AccountID,
    prior_balance: XRPAmount,
    issue: &MPTIssue,
) -> protocol::Ter {
    let mpt_id = issue.mpt_id();
    let issuance = match view.peek(protocol::mpt_issuance_keylet_from_mptid(mpt_id)) {
        Ok(Some(issuance)) => issuance,
        Ok(None) => return protocol::Ter::TEF_INTERNAL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    if issuance.get_field_u32(sf("sfFlags")) & protocol::lsfMPTLocked != 0 {
        return protocol::Ter::TEF_INTERNAL;
    }
    match view.peek(protocol::mptoken_keylet_from_mptid(
        mpt_id,
        to_uint160(*account),
    )) {
        Ok(Some(_)) => return protocol::Ter::TEC_DUPLICATE,
        Ok(None) => {}
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    }
    if *account == issue.issuer() {
        return protocol::Ter::TES_SUCCESS;
    }

    let acct_sle = match view.peek(account_keylet(to_uint160(*account))) {
        Ok(Some(acct_sle)) => acct_sle,
        Ok(None) => return protocol::Ter::TEC_INTERNAL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    let sponsor = match effective_tx_reserve_sponsor(view, tx, &acct_sle) {
        Ok(sponsor) => sponsor,
        Err(ter) => return ter,
    };
    if (sponsor.is_some() || acct_sle.get_field_u32(sf("sfOwnerCount")) >= 2)
        && let Err(ter) = check_holding_reserve(
            view,
            Some(tx),
            &acct_sle,
            prior_balance,
            sponsor.as_ref(),
            protocol::Ter::TEC_INSUFFICIENT_RESERVE,
        )
    {
        return ter;
    }

    let token_keylet = protocol::mptoken_keylet_from_mptid(mpt_id, to_uint160(*account));
    let owner_dir = owner_dir_keylet(to_uint160(*account));
    let owner_node = match crate::dir_insert(
        view,
        &owner_dir,
        token_keylet.key,
        &crate::describe_owner_dir(*account),
    ) {
        Ok(Some(page)) => page,
        Ok(None) => return protocol::Ter::TEC_DIR_FULL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };

    let mut token = STLedgerEntry::new(token_keylet);
    token.set_account_id(sf("sfAccount"), *account);
    token.set_field_h192(sf("sfMPTokenIssuanceID"), mpt_id);
    token.set_field_u64(sf("sfMPTAmount"), 0);
    token.set_field_u32(sf("sfFlags"), 0);
    token.set_field_u64(sf("sfOwnerNode"), owner_node);
    if let Some(sponsor_sle) = sponsor.as_ref() {
        token.set_account_id(sf("sfSponsor"), sponsor_sle.get_account_id(sf("sfAccount")));
    }
    let token = Arc::new(token);
    if view.insert(Arc::clone(&token)).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if crate::increase_owner_count_for_object(view, &acct_sle, sponsor.as_ref()).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    protocol::Ter::TES_SUCCESS
}

fn remove_empty_mpt_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issue: &MPTIssue,
) -> protocol::Ter {
    let account_is_issuer = *account == issue.issuer();
    let token_keylet = protocol::mptoken_keylet_from_mptid(issue.mpt_id(), to_uint160(*account));
    let token_sle = match view.peek(token_keylet) {
        Ok(Some(token_sle)) => token_sle,
        Ok(None) => {
            return if account_is_issuer {
                protocol::Ter::TES_SUCCESS
            } else {
                protocol::Ter::TEC_OBJECT_NOT_FOUND
            };
        }
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    if token_sle.get_field_u64(sf("sfMPTAmount")) != 0 {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }
    if view.rules().enabled(&feature_id("fixCleanup3_1_3"))
        && token_sle.is_field_present(sf("sfLockedAmount"))
        && token_sle.get_field_u64(sf("sfLockedAmount")) != 0
    {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }
    if [
        "sfConfidentialBalanceInbox",
        "sfConfidentialBalanceSpending",
        "sfIssuerEncryptedBalance",
        "sfAuditorEncryptedBalance",
    ]
    .into_iter()
    .any(|field| token_sle.is_field_present(sf(field)))
    {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }
    if !matches!(
        crate::dir_remove(
            view,
            &owner_dir_keylet(to_uint160(*account)),
            token_sle.get_field_u64(sf("sfOwnerNode")),
            *token_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    let acct_sle = match view.peek(account_keylet(to_uint160(*account))) {
        Ok(Some(acct_sle)) => acct_sle,
        Ok(None) => return protocol::Ter::TEC_INTERNAL,
        Err(_) => return protocol::Ter::TEF_BAD_LEDGER,
    };
    if crate::decrease_owner_count_for_object(view, &acct_sle, &token_sle, 1).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if view.erase(token_sle).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if erase_empty_owner_dir_root(view, account).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    protocol::Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        FreezeHandling, account_funds, add_empty_holding_with_tx, remove_empty_holding, xrp_liquid,
    };
    use crate::{ApplyViewImpl, Fees, Ledger, LedgerHeader, ReadView};
    use basics::base_uint::{Uint160, Uint256};
    use protocol::{
        AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTIssue,
        STAmount, STLedgerEntry, STObject, STTx, Ter, TxType, XRPAmount, account_keylet,
        currency_from_string, feature_id, get_field_by_symbol, line, lsfMPTLocked, make_mpt_id,
        mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, sf_generic,
    };
    use shamap::item::SHAMapItem;
    use shamap::mutation::MutableTree;
    use shamap::sync::{SHAMapType, SyncState, SyncTree};
    use shamap::tree_node::SHAMapNodeType;

    fn sample_uint256(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    fn sample_account(fill: u8) -> Uint160 {
        Uint160::from_array([fill; 20])
    }

    fn to_account_id(account: Uint160) -> AccountID {
        AccountID::from_slice(account.data()).expect("account width should match")
    }

    fn build_ledger(entries: &[(Uint256, Vec<u8>)], fees: Fees) -> Ledger {
        let seq = 88;
        let mut tree = MutableTree::new(seq);
        for (key, payload) in entries {
            tree.add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(*key, payload.clone()),
            )
            .expect("state item should insert");
        }

        let mut ledger = Ledger::from_maps(
            LedgerHeader {
                seq,
                ..LedgerHeader::default()
            },
            SyncTree::from_root_with_type(
                tree.root(),
                SHAMapType::State,
                false,
                seq,
                SyncState::Immutable,
            ),
            SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
        );
        ledger.set_fees(fees);
        ledger
    }

    fn account_root_entry(account: Uint160, balance: u64, owner_count: u32) -> Vec<u8> {
        account_root_entry_with_flags(account, balance, owner_count, 0)
    }

    fn account_root_entry_with_flags(
        account: Uint160,
        balance: u64,
        owner_count: u32,
        flags: u32,
    ) -> Vec<u8> {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(account).key,
        );
        entry.set_account_id(get_field_by_symbol("sfAccount"), to_account_id(account));
        entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::new_native(balance, false),
        );
        entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
        if flags != 0 {
            entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        }
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x51));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.get_serializer().data().to_vec()
    }

    fn trustline_entry(low: Uint160, high: Uint160, currency: Currency, balance: i64) -> Vec<u8> {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::RippleState,
            line(to_account_id(low), to_account_id(high), currency).key,
        );
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x61));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(balance, 0).expect("trustline balance should normalize"),
                Issue::new(currency, to_account_id(low)),
            ),
        );
        entry.set_field_amount(
            get_field_by_symbol("sfLowLimit"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(100, 0).expect("low limit should normalize"),
                Issue::new(currency, to_account_id(low)),
            ),
        );
        entry.set_field_amount(
            get_field_by_symbol("sfHighLimit"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(100, 0).expect("high limit should normalize"),
                Issue::new(currency, to_account_id(high)),
            ),
        );
        entry.get_serializer().data().to_vec()
    }

    fn mpt_issuance_entry(
        issuer: AccountID,
        sequence: u32,
        max_amount: u64,
        outstanding: u64,
        flags: u32,
    ) -> Vec<u8> {
        let id = make_mpt_id(sequence, issuer);
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::MPTokenIssuance,
            mpt_issuance_keylet_from_mptid(id).key,
        );
        entry.set_account_id(get_field_by_symbol("sfIssuer"), issuer);
        entry.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
        entry.set_field_u64(get_field_by_symbol("sfMaximumAmount"), max_amount);
        entry.set_field_u64(get_field_by_symbol("sfOutstandingAmount"), outstanding);
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x71));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.get_serializer().data().to_vec()
    }

    fn mptoken_entry(id: protocol::MPTID, holder: AccountID, amount: u64, flags: u32) -> Vec<u8> {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::MPToken,
            mptoken_keylet_from_mptid(id, Uint160::from_slice(holder.data()).expect("holder")).key,
        );
        entry.set_account_id(get_field_by_symbol("sfAccount"), holder);
        entry.set_field_h192(get_field_by_symbol("sfMPTokenIssuanceID"), id);
        entry.set_field_u64(get_field_by_symbol("sfMPTAmount"), amount);
        entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x72));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.get_serializer().data().to_vec()
    }

    #[test]
    fn xrp_liquid_subtracts_reserve() {
        let account = sample_account(0x11);
        let ledger = build_ledger(
            &[(
                account_keylet(account).key,
                account_root_entry(account, 1_000, 2),
            )],
            Fees {
                base: 10,
                reserve: 200,
                increment: 50,
            },
        );

        let liquid = xrp_liquid(&ledger, to_account_id(account), 0)
            .expect("xrp liquid lookup should succeed");

        assert_eq!(liquid.drops(), 700);
    }

    #[test]
    fn xrp_liquid_treats_unrepresentable_ledger_reserve_as_no_liquidity() {
        let account = sample_account(0x12);
        let ledger = build_ledger(
            &[(
                account_keylet(account).key,
                account_root_entry(account, 1_000, 0),
            )],
            Fees {
                base: 10,
                reserve: u64::MAX,
                increment: 0,
            },
        );

        assert_eq!(
            xrp_liquid(&ledger, to_account_id(account), 0),
            Err(shamap::traversal::TraversalError::View)
        );
    }

    #[test]
    fn account_funds_returns_trustline_balance_for_non_issuer() {
        let low = sample_account(0x21);
        let high = sample_account(0x31);
        let currency = currency_from_string("USD");
        let issue = Issue::new(currency, to_account_id(high));
        let ledger = build_ledger(
            &[(
                line(to_account_id(low), to_account_id(high), currency).key,
                trustline_entry(low, high, currency, 77),
            )],
            Fees::default(),
        );

        let funds = account_funds(
            &ledger,
            to_account_id(low),
            &STAmount::from_iou_amount(
                get_field_by_symbol("sfTakerGets"),
                IOUAmount::from_parts(1, 0).expect("offer amount should normalize"),
                issue,
            ),
            FreezeHandling::IgnoreFreeze,
        )
        .expect("account funds lookup should succeed");

        assert_eq!(funds.issue(), issue);
        assert_eq!(
            funds.iou(),
            IOUAmount::from_parts(77, 0).expect("expected canonical amount")
        );
    }

    #[test]
    fn account_funds_returns_mpt_holder_balance_without_panicking() {
        let issuer = to_account_id(sample_account(0x41));
        let holder = to_account_id(sample_account(0x42));
        let id = make_mpt_id(3, issuer);
        let issue = MPTIssue::new(id);
        let ledger = build_ledger(
            &[
                (
                    mpt_issuance_keylet_from_mptid(id).key,
                    mpt_issuance_entry(issuer, 3, 1_000, 200, 0),
                ),
                (
                    mptoken_keylet_from_mptid(id, sample_account(0x42)).key,
                    mptoken_entry(id, holder, 77, 0),
                ),
            ],
            Fees::default(),
        );

        let funds = account_funds(
            &ledger,
            holder,
            &STAmount::from_mpt_amount(
                get_field_by_symbol("sfTakerGets"),
                MPTAmount::from_value(1),
                issue,
            ),
            FreezeHandling::IgnoreFreeze,
        )
        .expect("account funds lookup should succeed");

        assert_eq!(funds.asset(), protocol::Asset::from(issue));
        assert_eq!(funds.mpt(), MPTAmount::from_value(77));
    }

    #[test]
    fn account_funds_returns_bounded_mpt_issuer_capacity_and_honors_locks() {
        let issuer = to_account_id(sample_account(0x51));
        let holder = to_account_id(sample_account(0x52));
        let id = make_mpt_id(4, issuer);
        let issue = MPTIssue::new(id);
        let default_amount = STAmount::from_mpt_amount(
            get_field_by_symbol("sfTakerGets"),
            MPTAmount::from_value(1),
            issue,
        );

        let ledger = build_ledger(
            &[
                (
                    mpt_issuance_keylet_from_mptid(id).key,
                    mpt_issuance_entry(issuer, 4, 1_000, 225, 0),
                ),
                (
                    mptoken_keylet_from_mptid(id, sample_account(0x52)).key,
                    mptoken_entry(id, holder, 77, lsfMPTLocked),
                ),
            ],
            Fees::default(),
        );

        let issuer_funds = account_funds(
            &ledger,
            issuer,
            &default_amount,
            FreezeHandling::ZeroIfFrozen,
        )
        .expect("issuer funds lookup should succeed");
        assert_eq!(issuer_funds.mpt(), MPTAmount::from_value(775));

        let frozen_holder_funds = account_funds(
            &ledger,
            holder,
            &default_amount,
            FreezeHandling::ZeroIfFrozen,
        )
        .expect("holder funds lookup should succeed");
        assert_eq!(frozen_holder_funds.mpt(), MPTAmount::new());
    }

    #[test]
    fn sponsored_mpt_holding_tracks_object_and_both_owner_counts() {
        let issuer = to_account_id(sample_account(0x61));
        let holder = to_account_id(sample_account(0x62));
        let sponsor = to_account_id(sample_account(0x63));
        let id = make_mpt_id(7, issuer);
        let issue = MPTIssue::new(id);
        let mut ledger = build_ledger(
            &[
                (
                    account_keylet(sample_account(0x61)).key,
                    account_root_entry(sample_account(0x61), 10_000_000, 0),
                ),
                (
                    account_keylet(sample_account(0x62)).key,
                    account_root_entry(sample_account(0x62), 1, 2),
                ),
                (
                    account_keylet(sample_account(0x63)).key,
                    account_root_entry(sample_account(0x63), 10_000_000, 0),
                ),
                (
                    mpt_issuance_keylet_from_mptid(id).key,
                    mpt_issuance_entry(issuer, 7, 1_000, 0, protocol::lsfMPTCanTransfer),
                ),
            ],
            Fees {
                base: 10,
                reserve: 200,
                increment: 50,
            },
        );
        ledger.set_rules(protocol::Rules::new([
            feature_id("Sponsor"),
            feature_id("fixCleanup3_1_3"),
        ]));
        let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), holder);
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(
                get_field_by_symbol("sfSponsorFlags"),
                crate::SPF_SPONSOR_RESERVE,
            );
            tx.set_field_object(
                get_field_by_symbol("sfSponsorSignature"),
                STObject::new(get_field_by_symbol("sfSponsorSignature")),
            );
        });

        assert_eq!(
            add_empty_holding_with_tx(
                &mut view,
                &tx,
                &holder,
                protocol::XRPAmount::from_drops(1),
                &protocol::Asset::from(issue),
            ),
            Ter::TES_SUCCESS
        );
        let token = view
            .read(mptoken_keylet_from_mptid(id, sample_account(0x62)))
            .unwrap()
            .expect("sponsored MPToken");
        assert_eq!(
            token.get_account_id(get_field_by_symbol("sfSponsor")),
            sponsor
        );
        let holder_root = view
            .read(account_keylet(sample_account(0x62)))
            .unwrap()
            .unwrap();
        let sponsor_root = view
            .read(account_keylet(sample_account(0x63)))
            .unwrap()
            .unwrap();
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
            3
        );
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfSponsoredOwnerCount")),
            1
        );
        assert_eq!(
            sponsor_root.get_field_u32(get_field_by_symbol("sfSponsoringOwnerCount")),
            1
        );

        assert_eq!(
            remove_empty_holding(&mut view, &holder, &protocol::Asset::from(issue)),
            Ter::TES_SUCCESS
        );
        let holder_root = view
            .read(account_keylet(sample_account(0x62)))
            .unwrap()
            .unwrap();
        let sponsor_root = view
            .read(account_keylet(sample_account(0x63)))
            .unwrap()
            .unwrap();
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
            2
        );
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfSponsoredOwnerCount")),
            0
        );
        assert_eq!(
            sponsor_root.get_field_u32(get_field_by_symbol("sfSponsoringOwnerCount")),
            0
        );
    }

    #[test]
    fn sponsored_iou_holding_uses_receivers_reserve_side_and_sponsor_field() {
        let issuer_raw = sample_account(0x81);
        let holder_raw = sample_account(0x72);
        let sponsor_raw = sample_account(0x73);
        let issuer = to_account_id(issuer_raw);
        let holder = to_account_id(holder_raw);
        let sponsor = to_account_id(sponsor_raw);
        let currency = currency_from_string("USD");
        let issue = Issue::new(currency, issuer);
        let mut ledger = build_ledger(
            &[
                (
                    account_keylet(issuer_raw).key,
                    account_root_entry_with_flags(
                        issuer_raw,
                        10_000_000,
                        0,
                        protocol::lsfDefaultRipple,
                    ),
                ),
                (
                    account_keylet(holder_raw).key,
                    account_root_entry(holder_raw, 1, 0),
                ),
                (
                    account_keylet(sponsor_raw).key,
                    account_root_entry(sponsor_raw, 10_000_000, 0),
                ),
            ],
            Fees {
                base: 10,
                reserve: 200,
                increment: 50,
            },
        );
        ledger.set_rules(protocol::Rules::new([feature_id("Sponsor")]));
        let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), holder);
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(
                get_field_by_symbol("sfSponsorFlags"),
                crate::SPF_SPONSOR_RESERVE,
            );
            tx.set_field_object(
                get_field_by_symbol("sfSponsorSignature"),
                STObject::new(get_field_by_symbol("sfSponsorSignature")),
            );
        });

        assert_eq!(
            add_empty_holding_with_tx(
                &mut view,
                &tx,
                &holder,
                XRPAmount::from_drops(1),
                &protocol::Asset::from(issue),
            ),
            Ter::TES_SUCCESS
        );
        let line_keylet = line(holder, issuer, currency);
        let trust = view.read(line_keylet).unwrap().expect("sponsored line");
        assert_ne!(
            trust.get_field_u32(get_field_by_symbol("sfFlags")) & protocol::lsfLowReserve,
            0
        );
        assert_eq!(
            trust.get_field_u32(get_field_by_symbol("sfFlags")) & protocol::lsfHighReserve,
            0
        );
        assert_eq!(
            trust.get_account_id(get_field_by_symbol("sfLowSponsor")),
            sponsor
        );

        assert_eq!(
            remove_empty_holding(&mut view, &holder, &protocol::Asset::from(issue)),
            Ter::TES_SUCCESS
        );
        assert!(view.read(line_keylet).unwrap().is_none());
        let holder_root = view.read(account_keylet(holder_raw)).unwrap().unwrap();
        let sponsor_root = view.read(account_keylet(sponsor_raw)).unwrap().unwrap();
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
            0
        );
        assert_eq!(
            holder_root.get_field_u32(get_field_by_symbol("sfSponsoredOwnerCount")),
            0
        );
        assert_eq!(
            sponsor_root.get_field_u32(get_field_by_symbol("sfSponsoringOwnerCount")),
            0
        );
    }
}
