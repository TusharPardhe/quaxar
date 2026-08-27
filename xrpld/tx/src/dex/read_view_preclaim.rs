//! Immutable `ReadView` typed preclaim helpers for the DEX transaction family.
//!
//! These helpers are deliberately limited to transaction types whose complete
//! `preclaim(...)` paths can be evaluated from an immutable ledger view. They
//! do not call apply code, create sandboxes, or return a permissive result for
//! a transaction type they do not own.

use basics::base_uint::Uint160;
use ledger::ReadView;
use protocol::{
    AMM_LP_TOKEN_FLAG, AMM_TWO_ASSET_IF_EMPTY_FLAG, AMM_WITHDRAW_ALL_FLAG, AccountID, ApplyFlags,
    Asset, IOUAmount, MPTAmount, STAmount, STTx, Ter, TxType, XRPAmount, get_field_by_symbol,
    lsfAllowTrustLineClawback, lsfDefaultRipple, lsfDisallowIncomingTrustline, lsfGlobalFreeze,
    lsfHighAuth, lsfHighDeepFreeze, lsfHighFreeze, lsfLowAuth, lsfLowDeepFreeze, lsfLowFreeze,
    lsfMPTAuthorized, lsfMPTCanClawback, lsfMPTRequireAuth, lsfNoFreeze, lsfRequireAuth,
};

use crate::{
    AMMCreatePreclaimFacts, AMMDeletePreclaimFacts, AMMDepositPreclaimFacts, AMMVotePreclaimFacts,
    AMMWithdrawPreclaimFacts, AmmBidPreclaimFacts, AmmBidSlotPricePreclaimFacts,
    OfferCancelPreclaimFacts, run_amm_bid_preclaim, run_amm_create_preclaim_facts,
    run_amm_delete_preclaim_facts, run_amm_deposit_preclaim_facts, run_amm_vote_preclaim_facts,
    run_amm_withdraw_preclaim_facts, run_offer_cancel_preclaim,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn read_account<V: ReadView>(
    view: &V,
    account: AccountID,
) -> Result<Option<std::sync::Arc<protocol::STLedgerEntry>>, Ter> {
    view.read(account_keylet(account)).map_err(|_| read_error())
}

fn tx_asset(tx: &STTx, field: &'static protocol::SField) -> Asset {
    tx.get_field_issue(field).asset()
}

fn read_amm<V: ReadView>(
    view: &V,
    asset: Asset,
    asset2: Asset,
) -> Result<Option<std::sync::Arc<protocol::STLedgerEntry>>, Ter> {
    view.read(protocol::keylet::amm(asset, asset2))
        .map_err(|_| read_error())
}

fn xrp_liquid<V: ReadView>(
    view: &V,
    account: AccountID,
    owner_count_adjustment: u32,
) -> Result<XRPAmount, Ter> {
    let Some(account_sle) = read_account(view, account)? else {
        return Ok(XRPAmount::new());
    };
    let adjustment = i32::try_from(owner_count_adjustment).map_err(|_| Ter::TEF_BAD_LEDGER)?;
    let reserve = if ledger::is_pseudo_account(&account_sle) {
        0
    } else {
        let owner_count = view
            .owner_count_hook(account, ledger::OwnerCounts::from_sle(&account_sle))
            .count();
        i64::try_from(ledger::effective_account_reserve_with_owner_count(
            view.fees(),
            &account_sle,
            owner_count,
            adjustment,
            0,
        ))
        .map_err(|_| Ter::TEF_BAD_LEDGER)?
    };
    let balance = view.balance_hook_iou(
        account,
        protocol::xrp_account(),
        account_sle.get_field_amount(sf("sfBalance")),
    );
    Ok(XRPAmount::from_drops(
        balance.xrp().drops().saturating_sub(reserve).max(0),
    ))
}

fn issue_auth<V: ReadView>(
    view: &V,
    account: AccountID,
    issue: protocol::Issue,
) -> Result<Ter, Ter> {
    if issue.native() || issue.account == account {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(issuer) = read_account(view, issue.account)? else {
        return Ok(Ter::TES_SUCCESS);
    };
    if issuer.get_field_u32(sf("sfFlags")) & lsfRequireAuth == 0 {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(line) = view
        .read(protocol::line(account, issue.account, issue.currency))
        .map_err(|_| read_error())?
    else {
        return Ok(Ter::TEC_NO_LINE);
    };
    Ok(
        if line.get_field_u32(sf("sfFlags"))
            & if account > issue.account {
                lsfLowAuth
            } else {
                lsfHighAuth
            }
            != 0
        {
            Ter::TES_SUCCESS
        } else {
            Ter::TEC_NO_AUTH
        },
    )
}

fn asset_auth<V: ReadView>(view: &V, account: AccountID, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) => issue_auth(view, account, issue),
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt_with_type(
            view,
            &issue,
            &account,
            ledger::mptoken_helpers::MPTAuthType::Strong,
        )
        .map_err(|_| read_error()),
    }
}

fn asset_frozen<V: ReadView>(view: &V, account: AccountID, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let global_frozen = read_account(view, issue.account)?
                .is_some_and(|issuer| issuer.get_field_u32(sf("sfFlags")) & lsfGlobalFreeze != 0);
            // Only the issuer's side freezes the issuer's asset. A holder may
            // set its own trust-line freeze bit, but that must not make its
            // balance unavailable for OfferCreate funding.
            let individually_frozen = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
                .is_some_and(|line| {
                    line.get_field_u32(sf("sfFlags"))
                        & if issue.account > account {
                            lsfHighFreeze
                        } else {
                            lsfLowFreeze
                        }
                        != 0
                });
            Ok(if global_frozen || individually_frozen {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            })
        }
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn global_frozen<V: ReadView>(view: &V, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => Ok(
            if read_account(view, issue.account)?
                .is_some_and(|issuer| issuer.get_field_u32(sf("sfFlags")) & lsfGlobalFreeze != 0)
            {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            },
        ),
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_global_frozen_mpt(view, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn asset_funds<V: ReadView>(
    view: &V,
    account: AccountID,
    requested: &STAmount,
    zero_if_frozen_or_unauthorized: bool,
) -> Result<STAmount, Ter> {
    match requested.asset() {
        Asset::Issue(issue) if issue.native() => {
            Ok(STAmount::from_xrp_amount(xrp_liquid(view, account, 0)?))
        }
        Asset::Issue(issue) if issue.account == account => Ok(requested.clone()),
        Asset::Issue(issue) => {
            if zero_if_frozen_or_unauthorized
                && asset_frozen(view, account, Asset::Issue(issue))? != Ter::TES_SUCCESS
            {
                return Ok(requested.zeroed());
            }
            let Some(line) = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
            else {
                return Ok(requested.zeroed());
            };
            let mut amount = line.get_field_amount(sf("sfBalance"));
            if account > issue.account {
                amount.negate();
            }
            amount.set_issuer(issue.account);
            Ok(amount)
        }
        Asset::MPTIssue(issue) if issue.issuer() == account => {
            let Some(issuance) = view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| read_error())?
            else {
                return Ok(requested.zeroed());
            };
            let maximum = if issuance.is_field_present(sf("sfMaximumAmount")) {
                issuance.get_field_u64(sf("sfMaximumAmount"))
            } else {
                i64::MAX as u64
            };
            let available = maximum
                .saturating_sub(issuance.get_field_u64(sf("sfOutstandingAmount")))
                .min(i64::MAX as u64) as i64;
            Ok(
                if view.rules().enabled(&protocol::feature_id("MPTokensV2")) {
                    view.balance_hook_self_issue_mpt(issue, available)
                } else {
                    STAmount::from_mpt_amount(
                        sf("sfAmount"),
                        protocol::MPTAmount::from_value(available),
                        issue,
                    )
                },
            )
        }
        Asset::MPTIssue(issue) => {
            let Some(token) = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .map_err(|_| read_error())?
            else {
                return Ok(requested.zeroed());
            };
            if zero_if_frozen_or_unauthorized {
                if ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                    .map_err(|_| read_error())?
                {
                    return Ok(requested.zeroed());
                }
                if view
                    .rules()
                    .enabled(&protocol::feature_id("SingleAssetVault"))
                {
                    if ledger::mptoken_helpers::require_auth_mpt_with_type(
                        view,
                        &issue,
                        &account,
                        ledger::mptoken_helpers::MPTAuthType::Strong,
                    )
                    .map_err(|_| read_error())?
                        != Ter::TES_SUCCESS
                    {
                        return Ok(requested.zeroed());
                    }
                } else if let Some(issuance) = view
                    .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                    .map_err(|_| read_error())?
                    && issuance.is_flag(lsfMPTRequireAuth)
                    && !token.is_flag(lsfMPTAuthorized)
                {
                    return Ok(requested.zeroed());
                }
            }
            let amount = token.get_field_u64(sf("sfMPTAmount")).min(i64::MAX as u64) as i64;
            Ok(
                if view.rules().enabled(&protocol::feature_id("MPTokensV2")) {
                    view.balance_hook_mpt(account, issue, amount)
                } else {
                    STAmount::from_mpt_amount(
                        sf("sfAmount"),
                        protocol::MPTAmount::from_value(amount),
                        issue,
                    )
                },
            )
        }
    }
}

fn account_can_accept_offer_asset<V: ReadView>(
    view: &V,
    account: AccountID,
    asset: Asset,
    apply_flags: ApplyFlags,
) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let Some(issuer) = read_account(view, issue.account)? else {
                return Ok(if apply_flags.bits() & ApplyFlags::RETRY.bits() != 0 {
                    Ter::TER_NO_ACCOUNT
                } else {
                    Ter::TEC_NO_ISSUER
                });
            };
            let line = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?;
            if view
                .rules()
                .enabled(&protocol::feature_id("fixCleanup3_4_0"))
                && issuer.is_flag(lsfDisallowIncomingTrustline)
                && line.is_none()
            {
                return Ok(if apply_flags.bits() & ApplyFlags::RETRY.bits() != 0 {
                    Ter::TER_NO_LINE
                } else {
                    Ter::TEC_NO_LINE
                });
            }
            let Some(line) = line else {
                return Ok(
                    if issuer.get_field_u32(sf("sfFlags")) & lsfRequireAuth != 0 {
                        if apply_flags.bits() & ApplyFlags::RETRY.bits() != 0 {
                            Ter::TER_NO_LINE
                        } else {
                            Ter::TEC_NO_LINE
                        }
                    } else {
                        Ter::TES_SUCCESS
                    },
                );
            };
            if issuer.get_field_u32(sf("sfFlags")) & lsfRequireAuth != 0
                && line.get_field_u32(sf("sfFlags"))
                    & if account > issue.account {
                        lsfLowAuth
                    } else {
                        lsfHighAuth
                    }
                    == 0
            {
                return Ok(if apply_flags.bits() & ApplyFlags::RETRY.bits() != 0 {
                    Ter::TER_NO_AUTH
                } else {
                    Ter::TEC_NO_AUTH
                });
            }
            Ok(
                if line.get_field_u32(sf("sfFlags")) & (lsfLowDeepFreeze | lsfHighDeepFreeze) != 0 {
                    Ter::TEC_FROZEN
                } else {
                    Ter::TES_SUCCESS
                },
            )
        }
        Asset::MPTIssue(issue) => {
            let auth = asset_auth(view, account, Asset::MPTIssue(issue))?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            asset_frozen(view, account, Asset::MPTIssue(issue))
        }
    }
}

fn is_lp_token<V: ReadView>(view: &V, asset: Asset) -> Result<bool, Ter> {
    Ok(read_account(view, asset.issuer())?
        .is_some_and(|account| account.is_field_present(sf("sfAMMID"))))
}

fn is_pseudo_account<V: ReadView>(view: &V, account: AccountID) -> Result<bool, Ter> {
    Ok(read_account(view, account)?.is_some_and(|account| {
        [sf("sfAMMID"), sf("sfVaultID"), sf("sfLoanBrokerID")]
            .into_iter()
            .any(|field| account.is_field_present(field))
    }))
}

fn is_mpt_issuer_pseudo<V: ReadView>(view: &V, asset: Asset) -> Result<bool, Ter> {
    match asset {
        Asset::MPTIssue(issue) => is_pseudo_account(view, issue.issuer()),
        Asset::Issue(_) => Ok(false),
    }
}

fn no_default_ripple<V: ReadView>(view: &V, asset: Asset) -> Result<bool, Ter> {
    match asset {
        Asset::Issue(issue) if !issue.native() => Ok(read_account(view, issue.account)?
            .is_some_and(|issuer| issuer.get_field_u32(sf("sfFlags")) & lsfDefaultRipple == 0)),
        _ => Ok(false),
    }
}

fn clawback_disabled<V: ReadView>(view: &V, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => Ok(match read_account(view, issue.account)? {
            Some(issuer)
                if issuer.get_field_u32(sf("sfFlags")) & lsfAllowTrustLineClawback != 0 =>
            {
                Ter::TEC_NO_PERMISSION
            }
            Some(_) => Ter::TES_SUCCESS,
            None => Ter::TEC_INTERNAL,
        }),
        Asset::MPTIssue(issue) => Ok(
            match view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| read_error())?
            {
                Some(issuance) if issuance.is_flag(lsfMPTCanClawback) => Ter::TEC_NO_PERMISSION,
                Some(_) => Ter::TES_SUCCESS,
                None => Ter::TEC_INTERNAL,
            },
        ),
    }
}

/// Computes the LP balance using the same canonical trust-line orientation as
/// `ammLPHolds` in rippled. The AMM LP token is always an IOU issue.
fn lp_holds<V: ReadView>(
    view: &V,
    amm: &protocol::STLedgerEntry,
    account: AccountID,
) -> Result<protocol::STAmount, Ter> {
    let lp_total = amm.get_field_amount(sf("sfLPTokenBalance"));
    let issue = lp_total.issue();
    let amm_account = amm.get_account_id(sf("sfAccount"));
    let Some(line) = view
        .read(protocol::line(account, amm_account, issue.currency))
        .map_err(|_| read_error())?
    else {
        return Ok(lp_total.zeroed());
    };
    let frozen_flag = if amm_account > account {
        lsfHighFreeze
    } else {
        lsfLowFreeze
    };
    if line.get_field_u32(sf("sfFlags")) & frozen_flag != 0 {
        return Ok(lp_total.zeroed());
    }

    let mut balance = line.get_field_amount(sf("sfBalance"));
    if account > amm_account {
        balance.negate();
    }
    balance.set_issuer(amm_account);
    Ok(balance)
}

fn amount_for_asset(asset: Asset) -> STAmount {
    match asset {
        Asset::Issue(issue) if issue.native() => STAmount::from_xrp_amount(XRPAmount::new()),
        Asset::Issue(issue) => STAmount::from_iou_amount(sf("sfAmount"), IOUAmount::new(), issue),
        Asset::MPTIssue(issue) => {
            STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::new(), issue)
        }
    }
}

fn amm_assets(amm: &protocol::STLedgerEntry) -> (Asset, Asset) {
    (
        amm.get_field_issue(sf("sfAsset")).asset(),
        amm.get_field_issue(sf("sfAsset2")).asset(),
    )
}

/// The `ammHolds` portion of the AMM deposit and withdraw preclaims.  The
/// optional requested assets preserve rippled's amount-driven pool ordering.
fn amm_holds<V: ReadView>(
    view: &V,
    amm: &protocol::STLedgerEntry,
    requested_asset: Option<Asset>,
    requested_asset2: Option<Asset>,
) -> Result<(STAmount, STAmount, STAmount), Ter> {
    let (asset, asset2) = amm_assets(amm);
    let (first, second) = match (requested_asset, requested_asset2) {
        (Some(first), Some(second)) => {
            if first == second
                || (first != asset && first != asset2)
                || (second != asset && second != asset2)
            {
                return Err(Ter::TEC_AMM_INVALID_TOKENS);
            }
            (first, second)
        }
        (Some(first), None) | (None, Some(first)) if first == asset => (asset, asset2),
        (Some(first), None) | (None, Some(first)) if first == asset2 => (asset2, asset),
        (Some(_), None) | (None, Some(_)) => return Err(Ter::TEC_AMM_INVALID_TOKENS),
        (None, None) => (asset, asset2),
    };
    let amm_account = amm.get_account_id(sf("sfAccount"));
    Ok((
        asset_funds(view, amm_account, &amount_for_asset(first), false)?,
        asset_funds(view, amm_account, &amount_for_asset(second), false)?,
        amm.get_field_amount(sf("sfLPTokenBalance")),
    ))
}

fn amm_asset_auth<V: ReadView>(
    view: &V,
    account: AccountID,
    asset: Asset,
    strong: bool,
) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let line = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?;
            if line.is_none() && strong {
                return Ok(Ter::TEC_NO_LINE);
            }
            if !read_account(view, issue.account)?.is_some_and(|sle| sle.is_flag(lsfRequireAuth)) {
                return Ok(Ter::TES_SUCCESS);
            }
            let Some(line) = line else {
                return Ok(Ter::TEC_NO_LINE);
            };
            Ok(
                if line.is_flag(if account > issue.account {
                    lsfLowAuth
                } else {
                    lsfHighAuth
                }) {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_NO_AUTH
                },
            )
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt_with_type(
            view,
            &issue,
            &account,
            if strong {
                ledger::mptoken_helpers::MPTAuthType::Strong
            } else {
                ledger::mptoken_helpers::MPTAuthType::Weak
            },
        )
        .map_err(|_| read_error()),
    }
}

fn individually_frozen<V: ReadView>(
    view: &V,
    account: AccountID,
    asset: Asset,
) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => Ok(
            if view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
                .is_some_and(|line| {
                    line.is_flag(if account > issue.account {
                        lsfHighFreeze
                    } else {
                        lsfLowFreeze
                    })
                })
            {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            },
        ),
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_individual_frozen_mpt(view, &account, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn deep_frozen<V: ReadView>(view: &V, account: AccountID, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => Ok(
            if view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
                .is_some_and(|line| {
                    line.is_flag(protocol::lsfHighDeepFreeze)
                        || line.is_flag(protocol::lsfLowDeepFreeze)
                })
            {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            },
        ),
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn check_deposit_freeze<V: ReadView>(
    view: &V,
    source: AccountID,
    pseudo: AccountID,
    asset: Asset,
) -> Result<Ter, Ter> {
    let global = global_frozen(view, asset)?;
    if global != Ter::TES_SUCCESS {
        return Ok(global);
    }
    if source != asset.issuer() {
        let source_frozen = individually_frozen(view, source, asset)?;
        if source_frozen != Ter::TES_SUCCESS {
            return Ok(source_frozen);
        }
    }
    individually_frozen(view, pseudo, asset)
}

fn check_withdraw_freeze<V: ReadView>(
    view: &V,
    pseudo: AccountID,
    submitter: AccountID,
    destination: AccountID,
    asset: Asset,
) -> Result<Ter, Ter> {
    if destination == asset.issuer() {
        return Ok(Ter::TES_SUCCESS);
    }
    let global = global_frozen(view, asset)?;
    if global != Ter::TES_SUCCESS {
        return Ok(global);
    }
    let pseudo_frozen = individually_frozen(view, pseudo, asset)?;
    if pseudo_frozen != Ter::TES_SUCCESS {
        return Ok(pseudo_frozen);
    }
    if submitter != destination {
        let submitter_frozen = individually_frozen(view, submitter, asset)?;
        if submitter_frozen != Ter::TES_SUCCESS {
            return Ok(submitter_frozen);
        }
    }
    deep_frozen(view, destination, asset)
}

fn deposit_balance<V: ReadView>(
    view: &V,
    account: AccountID,
    amm: &protocol::STLedgerEntry,
    deposit: &STAmount,
) -> Result<Ter, Ter> {
    if matches!(deposit.asset(), Asset::Issue(issue) if issue.native()) {
        let lp_issue = amm.get_field_amount(sf("sfLPTokenBalance")).issue();
        let has_lp_line = view
            .read(protocol::line(
                account,
                amm.get_account_id(sf("sfAccount")),
                lp_issue.currency,
            ))
            .map_err(|_| read_error())?
            .is_some();
        if xrp_liquid(view, account, u32::from(!has_lp_line))? >= deposit.xrp() {
            return Ok(Ter::TES_SUCCESS);
        }
        return Ok(if has_lp_line {
            Ter::TEC_UNFUNDED_AMM
        } else {
            Ter::TEC_INSUF_RESERVE_LINE
        });
    }
    Ok(if asset_funds(view, account, deposit, false)? >= *deposit {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_UNFUNDED_AMM
    })
}

fn deposit_amount_check<V: ReadView>(
    view: &V,
    account: AccountID,
    amm: &protocol::STLedgerEntry,
    amount: Option<&STAmount>,
    check_balance: bool,
) -> Result<Ter, Ter> {
    let Some(amount) = amount else {
        return Ok(Ter::TES_SUCCESS);
    };
    let result = amm_asset_auth(view, account, amount.asset(), true)?;
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    if !view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_3_0"))
    {
        let result = asset_frozen(view, amm.get_account_id(sf("sfAccount")), amount.asset())?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        let result = individually_frozen(view, account, amount.asset())?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }
    if check_balance {
        deposit_balance(view, account, amm, amount)
    } else {
        Ok(Ter::TES_SUCCESS)
    }
}

fn withdraw_amount_check<V: ReadView>(
    view: &V,
    account: AccountID,
    amm: &protocol::STLedgerEntry,
    amount: Option<&STAmount>,
    balance: &STAmount,
) -> Result<Ter, Ter> {
    let Some(amount) = amount else {
        return Ok(Ter::TES_SUCCESS);
    };
    if amount > balance {
        return Ok(Ter::TEC_AMM_BALANCE);
    }
    let result = amm_asset_auth(view, account, amount.asset(), false)?;
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    if view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_3_0"))
    {
        check_withdraw_freeze(
            view,
            amm.get_account_id(sf("sfAccount")),
            account,
            account,
            amount.asset(),
        )
    } else {
        let result = asset_frozen(view, amm.get_account_id(sf("sfAccount")), amount.asset())?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        individually_frozen(view, account, amount.asset())
    }
}

fn preclaim_offer_create<V: ReadView>(
    view: &V,
    tx: &STTx,
    apply_flags: ApplyFlags,
    account_sequence_floor: Option<u32>,
) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let taker_pays = tx.get_field_amount(sf("sfTakerPays"));
    let taker_gets = tx.get_field_amount(sf("sfTakerGets"));
    let Some(account_sle) = read_account(view, account)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };

    for asset in [taker_pays.asset(), taker_gets.asset()] {
        let result = global_frozen(view, asset)?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    let funds = asset_funds(view, account, &taker_gets, true)?;
    if !matches!(taker_gets.asset(), Asset::MPTIssue(issue) if issue.issuer() == account)
        && funds.signum() <= 0
    {
        return Ok(Ter::TEC_UNFUNDED_OFFER);
    }

    // Normal application supplies no floor, so this is exactly rippled's
    // `uAccountSequence <= OfferSequence` preclaim test. Direct dispatcher
    // callers deliberately omit the common sequence-consumption preamble;
    // they can provide their transaction sequence as a floor to represent the
    // logical account sequence without ever lowering a real ledger sequence.
    let account_sequence = account_sle
        .get_field_u32(sf("sfSequence"))
        .max(account_sequence_floor.unwrap_or_default());
    if tx.is_field_present(sf("sfOfferSequence"))
        && account_sequence <= tx.get_field_u32(sf("sfOfferSequence"))
    {
        return Ok(Ter::TEM_BAD_SEQUENCE);
    }

    if ledger::has_expired(
        view,
        tx.is_field_present(sf("sfExpiration"))
            .then(|| tx.get_field_u32(sf("sfExpiration"))),
    ) {
        return Ok(Ter::TEC_EXPIRED);
    }

    let acceptance =
        account_can_accept_offer_asset(view, account, taker_pays.asset(), apply_flags)?;
    if acceptance != Ter::TES_SUCCESS {
        return Ok(acceptance);
    }

    if tx.is_field_present(sf("sfDomainID"))
        && !ledger::permissioned_dex_helpers::account_in_domain(
            view,
            &account,
            &tx.get_field_h256(sf("sfDomainID")),
        )
        .map_err(|_| read_error())?
    {
        return Ok(Ter::TEC_NO_PERMISSION);
    }

    for asset in [taker_pays.asset(), taker_gets.asset()] {
        let result = ledger::mptoken_helpers::can_trade(view, &asset).map_err(|_| read_error())?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

fn preclaim_amm_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let amount2 = tx.get_field_amount(sf("sfAmount2"));
    let asset = amount.asset();
    let asset2 = amount2.asset();
    let amm_keylet = protocol::keylet::amm(asset, asset2);
    let single_asset_vault_enabled = view
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"));
    let mut facts = AMMCreatePreclaimFacts {
        amm_exists: view.read(amm_keylet).map_err(|_| read_error())?.is_some(),
        single_asset_vault_enabled,
        ..Default::default()
    };
    let mut result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    facts.amount_auth_result = asset_auth(view, account, asset)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_auth_result = asset_auth(view, account, asset2)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount_frozen_result = asset_frozen(view, account, asset)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_frozen_result = asset_frozen(view, account, asset2)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount_no_default_ripple = no_default_ripple(view, asset)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_no_default_ripple = no_default_ripple(view, asset2)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    let liquid = xrp_liquid(view, account, 1)?;
    facts.xrp_reserve_positive = liquid.drops() > 0;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount_insufficient_balance = if amount.native() {
        STAmount::from_xrp_amount(liquid) < amount
    } else {
        asset_funds(view, account, &amount, true)? < amount
    };
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_insufficient_balance = if amount2.native() {
        STAmount::from_xrp_amount(liquid) < amount2
    } else {
        asset_funds(view, account, &amount2, true)? < amount2
    };
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount_is_lp_token = is_lp_token(view, asset)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_is_lp_token = is_lp_token(view, asset2)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    if single_asset_vault_enabled {
        facts.address_collision = ledger::pseudo_account_address(view, amm_keylet.key)
            .map_err(|_| read_error())?
            .is_zero();
        result = run_amm_create_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.amount_is_vault_share = is_mpt_issuer_pseudo(view, asset)?;
        result = run_amm_create_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.amount2_is_vault_share = is_mpt_issuer_pseudo(view, asset2)?;
        result = run_amm_create_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    facts.amount_mpt_trade_transfer_result =
        ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset, &account, &account)
            .map_err(|_| read_error())?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_mpt_trade_transfer_result =
        ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset2, &account, &account)
            .map_err(|_| read_error())?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    facts.amm_clawback_enabled = view.rules().enabled(&protocol::feature_id("AMMClawback"));
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS || facts.amm_clawback_enabled {
        return Ok(result);
    }
    facts.amount_clawback_disabled_result = clawback_disabled(view, asset)?;
    result = run_amm_create_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_clawback_disabled_result = clawback_disabled(view, asset2)?;
    Ok(run_amm_create_preclaim_facts(facts))
}

fn preclaim_offer_cancel<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let account_root = read_account(view, account)?;
    Ok(run_offer_cancel_preclaim(OfferCancelPreclaimFacts {
        account_exists: account_root.is_some(),
        account_sequence: account_root
            .as_ref()
            .map_or(0, |sle| sle.get_field_u32(sf("sfSequence"))),
        offer_sequence: tx.get_field_u32(sf("sfOfferSequence")),
    }))
}

fn preclaim_amm_vote<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let Some(amm) = read_amm(view, asset, asset2)? else {
        return Ok(run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: false,
            lp_token_balance_signum: 0,
            account_lp_holds_signum: None,
        }));
    };
    let lp_token_balance_signum = amm.get_field_amount(sf("sfLPTokenBalance")).signum();
    let front = run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
        amm_exists: true,
        lp_token_balance_signum,
        account_lp_holds_signum: None,
    });
    if front != Ter::TES_SUCCESS {
        return Ok(front);
    }
    let account = tx.get_account_id(sf("sfAccount"));
    let lp_holds = lp_holds(view, &amm, account)?;
    Ok(run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
        amm_exists: true,
        lp_token_balance_signum,
        account_lp_holds_signum: Some(lp_holds.signum()),
    }))
}

fn slot_price_facts(
    price: protocol::STAmount,
    lp_tokens: &protocol::STAmount,
    lp_total: &protocol::STAmount,
) -> AmmBidSlotPricePreclaimFacts {
    AmmBidSlotPricePreclaimFacts {
        issue_matches_lp_token: price.asset() == lp_tokens.asset(),
        exceeds_lp_tokens: price > *lp_tokens,
        reaches_pool_balance: price >= *lp_total,
    }
}

fn preclaim_amm_bid<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let Some(amm) = read_amm(view, asset, asset2)? else {
        return Ok(run_amm_bid_preclaim(AmmBidPreclaimFacts {
            amm_exists: false,
            lp_token_balance_is_zero: false,
            auth_accounts_exist: Vec::new(),
            lp_tokens_is_zero: true,
            bid_min: None,
            bid_max: None,
            bid_min_exceeds_bid_max: false,
        }));
    };

    let lp_total = amm.get_field_amount(sf("sfLPTokenBalance"));
    if lp_total.signum() == 0 {
        return Ok(Ter::TEC_AMM_EMPTY);
    }
    let mut auth_accounts = Vec::new();
    if tx.is_field_present(sf("sfAuthAccounts")) {
        for entry in tx.get_field_array(sf("sfAuthAccounts")).iter() {
            let exists = read_account(view, entry.get_account_id(sf("sfAccount")))?.is_some();
            auth_accounts.push(exists);
            if !exists {
                return Ok(Ter::TER_NO_ACCOUNT);
            }
        }
    }
    let account = tx.get_account_id(sf("sfAccount"));
    let lp_tokens = lp_holds(view, &amm, account)?;
    if lp_tokens.signum() == 0 {
        return Ok(Ter::TEC_AMM_INVALID_TOKENS);
    }
    let bid_min = tx
        .is_field_present(sf("sfBidMin"))
        .then(|| slot_price_facts(tx.get_field_amount(sf("sfBidMin")), &lp_tokens, &lp_total));
    let bid_max = tx
        .is_field_present(sf("sfBidMax"))
        .then(|| slot_price_facts(tx.get_field_amount(sf("sfBidMax")), &lp_tokens, &lp_total));
    let bid_min_exceeds_bid_max = tx.is_field_present(sf("sfBidMin"))
        && tx.is_field_present(sf("sfBidMax"))
        && tx.get_field_amount(sf("sfBidMin")) > tx.get_field_amount(sf("sfBidMax"));

    Ok(run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: lp_total.signum() == 0,
        auth_accounts_exist: auth_accounts,
        lp_tokens_is_zero: lp_tokens.signum() == 0,
        bid_min,
        bid_max,
        bid_min_exceeds_bid_max,
    }))
}

fn preclaim_amm_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let amm = read_amm(view, asset, asset2)?;
    Ok(run_amm_delete_preclaim_facts(AMMDeletePreclaimFacts {
        amm_exists: amm.is_some(),
        lp_token_balance_is_zero: amm
            .as_ref()
            .is_none_or(|sle| sle.get_field_amount(sf("sfLPTokenBalance")).signum() == 0),
    }))
}

fn preclaim_amm_deposit<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let Some(amm) = read_amm(view, asset, asset2)? else {
        return Ok(run_amm_deposit_preclaim_facts(AMMDepositPreclaimFacts {
            amm_exists: false,
            ..Default::default()
        }));
    };
    let (amount_balance, amount2_balance, lp_token_balance) =
        match amm_holds(view, &amm, None, None) {
            Ok(holds) => holds,
            Err(ter) => {
                return Ok(run_amm_deposit_preclaim_facts(AMMDepositPreclaimFacts {
                    amm_exists: true,
                    amm_holds_result: ter,
                    ..Default::default()
                }));
            }
        };
    let mut facts = AMMDepositPreclaimFacts {
        amm_exists: true,
        amm_holds_result: Ter::TES_SUCCESS,
        two_asset_if_empty: tx.is_flag(AMM_TWO_ASSET_IF_EMPTY_FLAG),
        amount_balance_signum: amount_balance.signum(),
        amount2_balance_signum: amount2_balance.signum(),
        lp_token_balance_signum: lp_token_balance.signum(),
        amm_clawback_enabled: view.rules().enabled(&protocol::feature_id("AMMClawback"))
            || view
                .rules()
                .enabled(&protocol::feature_id("fixCleanup3_3_0")),
        ..Default::default()
    };
    let result = run_amm_deposit_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    if facts.amm_clawback_enabled {
        facts.asset_auth_result = amm_asset_auth(view, account, asset, false)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.asset_frozen_result = if view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_3_0"))
        {
            check_deposit_freeze(view, account, amm.get_account_id(sf("sfAccount")), asset)?
        } else {
            asset_frozen(view, account, asset)?
        };
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.asset2_auth_result = amm_asset_auth(view, account, asset2, false)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.asset2_frozen_result = if view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_3_0"))
        {
            check_deposit_freeze(view, account, amm.get_account_id(sf("sfAccount")), asset2)?
        } else {
            asset_frozen(view, account, asset2)?
        };
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    let amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let amount2 = tx
        .is_field_present(sf("sfAmount2"))
        .then(|| tx.get_field_amount(sf("sfAmount2")));
    if !tx.is_flag(AMM_LP_TOKEN_FLAG) {
        facts.amount_check_result =
            deposit_amount_check(view, account, &amm, amount.as_ref(), true)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.amount2_check_result =
            deposit_amount_check(view, account, &amm, amount2.as_ref(), true)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    } else {
        facts.lp_token_mode = true;
        facts.pool_amount_check_result =
            deposit_amount_check(view, account, &amm, Some(&amount_balance), false)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.pool_amount2_check_result =
            deposit_amount_check(view, account, &amm, Some(&amount2_balance), false)?;
        let result = run_amm_deposit_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    facts.lp_token_out_asset_matches_lpt = tx
        .is_field_present(sf("sfLPTokenOut"))
        .then(|| tx.get_field_amount(sf("sfLPTokenOut")).asset() == lp_token_balance.asset());
    let result = run_amm_deposit_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    let lp_tokens = lp_holds(view, &amm, account)?;
    facts.account_lp_holds_signum = lp_tokens.signum();
    facts.xrp_reserve_positive = if lp_tokens.signum() == 0 {
        xrp_liquid(view, account, 1)?.drops() > 0
    } else {
        true
    };
    let result = run_amm_deposit_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    facts.asset_mpt_trade_transfer_result =
        ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset, &account, &account)
            .map_err(|_| read_error())?;
    let result = run_amm_deposit_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.asset2_mpt_trade_transfer_result =
        ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset2, &account, &account)
            .map_err(|_| read_error())?;
    Ok(run_amm_deposit_preclaim_facts(facts))
}

fn preclaim_amm_withdraw<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let Some(amm) = read_amm(view, asset, asset2)? else {
        return Ok(run_amm_withdraw_preclaim_facts(AMMWithdrawPreclaimFacts {
            amm_exists: false,
            ..Default::default()
        }));
    };
    let amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let amount2 = tx
        .is_field_present(sf("sfAmount2"))
        .then(|| tx.get_field_amount(sf("sfAmount2")));
    let (amount_balance, amount2_balance, lp_token_balance) = match amm_holds(
        view,
        &amm,
        amount.as_ref().map(STAmount::asset),
        amount2.as_ref().map(STAmount::asset),
    ) {
        Ok(holds) => holds,
        Err(ter) => {
            return Ok(run_amm_withdraw_preclaim_facts(AMMWithdrawPreclaimFacts {
                amm_exists: true,
                amm_holds_result: ter,
                ..Default::default()
            }));
        }
    };
    let mut facts = AMMWithdrawPreclaimFacts {
        amm_exists: true,
        amm_holds_result: Ter::TES_SUCCESS,
        amount_balance_signum: amount_balance.signum(),
        amount2_balance_signum: amount2_balance.signum(),
        lp_token_balance_signum: lp_token_balance.signum(),
        ..Default::default()
    };
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    facts.amount_check_result =
        withdraw_amount_check(view, account, &amm, amount.as_ref(), &amount_balance)?;
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.amount2_check_result =
        withdraw_amount_check(view, account, &amm, amount2.as_ref(), &amount2_balance)?;
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    let lp_tokens = lp_holds(view, &amm, account)?;
    let lp_tokens_withdraw = if (tx.get_flags()
        & (AMM_WITHDRAW_ALL_FLAG | protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG))
        != 0
    {
        Some(lp_tokens.clone())
    } else {
        tx.is_field_present(sf("sfLPTokenIn"))
            .then(|| tx.get_field_amount(sf("sfLPTokenIn")))
    };
    facts.account_lp_tokens_signum = lp_tokens.signum();
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.lp_tokens_withdraw_asset_matches_lp = lp_tokens_withdraw
        .as_ref()
        .map(|tokens| tokens.asset() == lp_tokens.asset());
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.lp_tokens_withdraw_exceeds_balance = lp_tokens_withdraw
        .as_ref()
        .is_some_and(|tokens| tokens > &lp_tokens);
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }
    facts.e_price_asset_matches_lp = tx
        .is_field_present(sf("sfEPrice"))
        .then(|| tx.get_field_amount(sf("sfEPrice")).asset() == lp_tokens.asset());
    let result = run_amm_withdraw_preclaim_facts(facts);
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    facts.lp_token_or_withdraw_all_mode =
        (tx.get_flags() & (AMM_LP_TOKEN_FLAG | AMM_WITHDRAW_ALL_FLAG)) != 0;
    if facts.lp_token_or_withdraw_all_mode {
        facts.pool_amount_check_result =
            withdraw_amount_check(view, account, &amm, Some(&amount_balance), &amount_balance)?;
        let result = run_amm_withdraw_preclaim_facts(facts);
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        facts.pool_amount2_check_result = withdraw_amount_check(
            view,
            account,
            &amm,
            Some(&amount2_balance),
            &amount2_balance,
        )?;
    }
    Ok(run_amm_withdraw_preclaim_facts(facts))
}

fn clawback_asset_allowed<V: ReadView>(
    view: &V,
    issuer: AccountID,
    issuer_flags: u32,
    asset: Asset,
) -> Result<bool, Ter> {
    match asset {
        Asset::Issue(issue) => Ok(!issue.native()
            && (issuer_flags & lsfAllowTrustLineClawback) != 0
            && (issuer_flags & lsfNoFreeze) == 0),
        Asset::MPTIssue(issue) => Ok(view
            .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .map_err(|_| read_error())?
            .is_some_and(|issuance| {
                issuance.is_flag(lsfMPTCanClawback)
                    && issuance.get_account_id(sf("sfIssuer")) == issuer
            })),
    }
}

fn preclaim_amm_clawback<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let issuer = tx.get_account_id(sf("sfAccount"));
    let holder = tx.get_account_id(sf("sfHolder"));
    let asset = tx_asset(tx, sf("sfAsset"));
    let asset2 = tx_asset(tx, sf("sfAsset2"));
    let Some(issuer_sle) = read_account(view, issuer)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    if read_account(view, holder)?.is_none() {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    if read_amm(view, asset, asset2)?.is_none() {
        return Ok(Ter::TER_NO_AMM);
    }

    let issuer_flags = issuer_sle.get_field_u32(sf("sfFlags"));
    let mptokens_v2_enabled = view.rules().enabled(&protocol::feature_id("MPTokensV2"));
    // AMMClawback::preclaim performs the legacy issuer permission check
    // before checkClawAsset. In particular, without MPTokensV2 an MPT
    // issuance must not be read before this canonical tecNO_PERMISSION.
    if !mptokens_v2_enabled
        && ((issuer_flags & lsfAllowTrustLineClawback) == 0 || (issuer_flags & lsfNoFreeze) != 0)
    {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    let claw_two_assets = (tx.get_flags() & protocol::AMM_CLAWBACK_TWO_ASSETS_FLAG) != 0;
    if !clawback_asset_allowed(view, issuer, issuer_flags, asset)? {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    if claw_two_assets && !clawback_asset_allowed(view, issuer, issuer_flags, asset2)? {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    Ok(Ter::TES_SUCCESS)
}

/// Runs OfferCreate preclaim for direct state-dispatch callers that bypass
/// the common sequence-consumption preamble. Only a conventional Sequence
/// transaction gets a non-lowering logical account-sequence floor. A ticket
/// does not advance AccountRoot.sfSequence, so ticket transactions must retain
/// the canonical AccountRoot-only comparison used by rippled preclaim.
///
/// Production application must use `run_dex_read_view_preclaim_*`, which
/// always reads only AccountRoot.sfSequence before the common preamble.
pub fn run_offer_create_direct_dispatch_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    apply_flags: ApplyFlags,
) -> Ter {
    let sequence_floor = tx
        .get_seq_proxy()
        .is_seq()
        .then(|| tx.get_seq_proxy().value());
    preclaim_offer_create(view, tx, apply_flags, sequence_floor).unwrap_or_else(|ter| ter)
}

/// Evaluates the owned DEX preclaim tail with explicit apply flags.
///
/// `None` means the type is not owned by this helper. In particular it never
/// means success and callers must continue to an explicit typed helper.
pub fn run_dex_read_view_preclaim_with_flags<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
    apply_flags: ApplyFlags,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::OFFER_CREATE => preclaim_offer_create(view, tx, apply_flags, None),
        TxType::OFFER_CANCEL => preclaim_offer_cancel(view, tx),
        TxType::AMM_CREATE => preclaim_amm_create(view, tx),
        TxType::AMM_DEPOSIT => preclaim_amm_deposit(view, tx),
        TxType::AMM_WITHDRAW => preclaim_amm_withdraw(view, tx),
        TxType::AMM_VOTE => preclaim_amm_vote(view, tx),
        TxType::AMM_BID => preclaim_amm_bid(view, tx),
        TxType::AMM_DELETE => preclaim_amm_delete(view, tx),
        TxType::AMM_CLAWBACK => preclaim_amm_clawback(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

/// Evaluates the owned DEX preclaim tail against `view` without retry mode.
///
/// `None` means the type is not owned by this helper. In particular it never
/// means success and callers must continue to an explicit typed helper.
pub fn run_dex_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    run_dex_read_view_preclaim_with_flags(view, tx, txn_type, ApplyFlags::NONE)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        AccountID, ApplyFlags, Asset, Currency, IOUAmount, Issue, LedgerEntryType, STAmount,
        STLedgerEntry, STTx, Ter, TxType, XRPAmount, get_field_by_symbol,
    };

    use super::{
        account_can_accept_offer_asset, is_mpt_issuer_pseudo, run_dex_read_view_preclaim,
        run_dex_read_view_preclaim_with_flags, run_offer_create_direct_dispatch_preclaim,
    };

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        fail_reads: BTreeSet<Uint256>,
        rules: Rules,
    }

    impl View {
        fn insert(&mut self, entry: STLedgerEntry) {
            self.entries.insert(*entry.key(), Arc::new(entry));
        }
    }

    impl ReadView for View {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            self.rules.clone()
        }
        fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(
            &self,
            _key: Uint256,
            _last: Option<Uint256>,
        ) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            if self.fail_reads.contains(&keylet.key) {
                return Err(ViewError::Conversion("fault-injected DEX read".into()));
            }
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _key: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    fn account_entry(id: AccountID, sequence: u32) -> STLedgerEntry {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            protocol::account_keylet(basics::base_uint::Uint160::from_void(id.data())).key,
        );
        entry.set_account_id(sf("sfAccount"), id);
        entry.set_field_u32(sf("sfSequence"), sequence);
        entry.set_field_u32(sf("sfFlags"), 0);
        entry.set_field_u32(sf("sfOwnerCount"), 0);
        entry.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
        entry
    }

    fn issue(fill: u8, issuer: AccountID) -> Issue {
        Issue::new(Currency::from_array([fill; 20]), issuer)
    }

    #[test]
    fn amm_create_identifies_mpt_assets_issued_by_pseudo_accounts() {
        let issuer = account(3);
        let mut pseudo = account_entry(issuer, 1);
        pseudo.set_field_h256(sf("sfVaultID"), Uint256::from_array([9; 32]));
        let mut view = View::default();
        view.insert(pseudo);

        let mpt = Asset::MPTIssue(protocol::MPTIssue::new(protocol::make_mpt_id(1, issuer)));
        assert_eq!(is_mpt_issuer_pseudo(&view, mpt), Ok(true));
        assert_eq!(
            is_mpt_issuer_pseudo(&view, Asset::Issue(issue(4, issuer))),
            Ok(false)
        );
    }

    #[test]
    fn offer_acceptance_honors_disallow_incoming_trustline_only_after_cleanup_3_4() {
        let owner = account(1);
        let issuer = account(2);
        let asset = Asset::Issue(issue(3, issuer));
        let mut issuer_entry = account_entry(issuer, 1);
        issuer_entry.set_field_u32(sf("sfFlags"), protocol::lsfDisallowIncomingTrustline);

        let mut legacy = View::default();
        legacy.insert(issuer_entry.clone());
        assert_eq!(
            account_can_accept_offer_asset(&legacy, owner, asset, ApplyFlags::NONE),
            Ok(Ter::TES_SUCCESS)
        );

        let mut fixed = View {
            rules: Rules::new([protocol::feature_id("fixCleanup3_4_0")]),
            ..View::default()
        };
        fixed.insert(issuer_entry);
        assert_eq!(
            account_can_accept_offer_asset(&fixed, owner, asset, ApplyFlags::NONE),
            Ok(Ter::TEC_NO_LINE)
        );
        assert_eq!(
            account_can_accept_offer_asset(&fixed, owner, asset, ApplyFlags::RETRY),
            Ok(Ter::TER_NO_LINE)
        );
    }

    fn amm_entry(amm_account: AccountID, asset: Asset, asset2: Asset, lp: i64) -> STLedgerEntry {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AMM,
            protocol::keylet::amm(asset, asset2).key,
        );
        entry.set_account_id(sf("sfAccount"), amm_account);
        entry.set_field_issue(
            sf("sfAsset"),
            protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
        );
        entry.set_field_issue(
            sf("sfAsset2"),
            protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
        );
        entry.set_field_amount(
            sf("sfLPTokenBalance"),
            STAmount::from_iou_amount(
                sf("sfLPTokenBalance"),
                IOUAmount::from_parts(lp, 0).expect("valid LP amount"),
                protocol::amm_lpt_issue_from_assets(asset, asset2, amm_account),
            ),
        );
        entry
    }

    #[test]
    fn offer_cancel_reads_account_sequence_and_has_no_unowned_success_default() {
        let owner = account(1);
        let mut view = View::default();
        view.insert(account_entry(owner, 7));
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), owner);
            tx.set_field_u32(sf("sfOfferSequence"), 7);
        });

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::OFFER_CANCEL),
            Some(protocol::Ter::TEM_BAD_SEQUENCE)
        );
        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::PAYMENT),
            None,
            "unowned types must not receive a success result"
        );
    }

    #[test]
    fn offer_create_direct_dispatch_sequence_allows_prior_offer_replacement() {
        let owner = account(1);
        let mut view = View::default();
        // Direct dispatcher tests apply the first transaction's OfferCreate
        // mutation but not its common account-sequence preamble. The second
        // transaction still carries its real sequence (2).
        view.insert(account_entry(owner, 1));
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), owner);
            tx.set_field_u32(sf("sfSequence"), 2);
            tx.set_field_u32(sf("sfOfferSequence"), 1);
            tx.set_field_amount(
                sf("sfTakerPays"),
                STAmount::from_iou_amount(
                    sf("sfTakerPays"),
                    IOUAmount::from_parts(1, 0).expect("valid offer amount"),
                    Issue::new(Currency::from_array([3; 20]), owner),
                ),
            );
            tx.set_field_amount(
                sf("sfTakerGets"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        });

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::OFFER_CREATE),
            Some(Ter::TEM_BAD_SEQUENCE),
            "normal application must use the AccountRoot sequence exactly"
        );
        assert_eq!(
            run_offer_create_direct_dispatch_preclaim(&view, &tx, ApplyFlags::NONE),
            Ter::TES_SUCCESS,
            "direct dispatch must model the logical transaction sequence"
        );
    }

    #[test]
    fn offer_create_direct_dispatch_ticket_keeps_account_sequence_check() {
        let owner = account(1);
        let mut view = View::default();
        view.insert(account_entry(owner, 1));
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), owner);
            tx.set_field_u32(sf("sfSequence"), 0);
            tx.set_field_u32(sf("sfTicketSequence"), 2);
            tx.set_field_u32(sf("sfOfferSequence"), 1);
            tx.set_field_amount(
                sf("sfTakerPays"),
                STAmount::from_iou_amount(
                    sf("sfTakerPays"),
                    IOUAmount::from_parts(1, 0).expect("valid offer amount"),
                    Issue::new(Currency::from_array([3; 20]), owner),
                ),
            );
            tx.set_field_amount(
                sf("sfTakerGets"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        });

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::OFFER_CREATE),
            Some(Ter::TEM_BAD_SEQUENCE),
            "normal ticket application compares OfferSequence to AccountRoot sequence"
        );
        assert_eq!(
            run_offer_create_direct_dispatch_preclaim(&view, &tx, ApplyFlags::NONE),
            Ter::TEM_BAD_SEQUENCE,
            "a ticket number must not relax OfferSequence validation"
        );
    }

    #[test]
    fn amm_delete_reads_empty_lp_supply_without_mutating_the_view() {
        let issuer = account(2);
        let asset = Asset::from(issue(3, issuer));
        let asset2 = Asset::from(issue(4, issuer));
        let mut view = View::default();
        view.insert(amm_entry(account(5), asset, asset2, 1));
        let tx = STTx::new(TxType::AMM_DELETE, |tx| {
            tx.set_field_issue(
                sf("sfAsset"),
                protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
            );
        });

        let before = view.entries.len();
        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::AMM_DELETE),
            Some(protocol::Ter::TEC_AMM_NOT_EMPTY)
        );
        assert_eq!(view.entries.len(), before, "preclaim is read-only");
    }

    #[test]
    fn offer_create_reads_issuer_and_preserves_retry_result_without_mutation() {
        let owner = account(1);
        let missing_issuer = account(2);
        let taker_pays = STAmount::from_iou_amount(
            sf("sfTakerPays"),
            IOUAmount::from_parts(1, 0).expect("valid offer amount"),
            Issue::new(Currency::from_array([3; 20]), missing_issuer),
        );
        let taker_gets = STAmount::from_xrp_amount(XRPAmount::from_drops(1));
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), owner);
            tx.set_field_amount(sf("sfTakerPays"), taker_pays.clone());
            tx.set_field_amount(sf("sfTakerGets"), taker_gets.clone());
        });
        let mut view = View::default();
        view.insert(account_entry(owner, 7));
        let before = view.entries.len();

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::OFFER_CREATE),
            Some(Ter::TEC_NO_ISSUER)
        );
        assert_eq!(
            run_dex_read_view_preclaim_with_flags(
                &view,
                &tx,
                TxType::OFFER_CREATE,
                ApplyFlags::RETRY,
            ),
            Some(Ter::TER_NO_ACCOUNT)
        );
        assert_eq!(view.entries.len(), before, "preclaim is read-only");
    }

    #[test]
    fn amm_create_reports_duplicate_before_later_read_view_facts_without_mutation() {
        let issuer = account(4);
        let asset = Asset::from(issue(5, issuer));
        let asset2 = Asset::from(issue(6, issuer));
        let amount = STAmount::from_iou_amount(
            sf("sfAmount"),
            IOUAmount::from_parts(1, 0).expect("valid AMM amount"),
            issue(5, issuer),
        );
        let amount2 = STAmount::from_iou_amount(
            sf("sfAmount2"),
            IOUAmount::from_parts(1, 0).expect("valid AMM amount"),
            issue(6, issuer),
        );
        let tx = STTx::new(TxType::AMM_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), account(7));
            tx.set_field_amount(sf("sfAmount"), amount.clone());
            tx.set_field_amount(sf("sfAmount2"), amount2.clone());
        });
        let mut view = View::default();
        view.insert(amm_entry(account(8), asset, asset2, 1));
        let before = view.entries.len();

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::AMM_CREATE),
            Some(Ter::TEC_DUPLICATE)
        );
        assert_eq!(view.entries.len(), before, "preclaim is read-only");
    }

    #[test]
    fn amm_deposit_and_withdraw_are_typed_read_view_preclaims() {
        let issuer = account(20);
        let asset = Asset::from(issue(21, issuer));
        let asset2 = Asset::from(issue(22, issuer));
        let account_id = account(23);
        let missing = STTx::new(TxType::AMM_DEPOSIT, |tx| {
            tx.set_account_id(sf("sfAccount"), account_id);
            tx.set_field_issue(
                sf("sfAsset"),
                protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
            );
        });
        assert_eq!(
            run_dex_read_view_preclaim(&View::default(), &missing, TxType::AMM_DEPOSIT),
            Some(Ter::TER_NO_AMM)
        );
        assert_eq!(
            run_dex_read_view_preclaim(&View::default(), &missing, TxType::AMM_WITHDRAW),
            Some(Ter::TER_NO_AMM)
        );

        let mut view = View::default();
        view.insert(amm_entry(account(24), asset, asset2, 1));
        let nonempty_deposit = STTx::new(TxType::AMM_DEPOSIT, |tx| {
            tx.set_account_id(sf("sfAccount"), account_id);
            tx.set_field_issue(
                sf("sfAsset"),
                protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
            );
            tx.set_field_u32(sf("sfFlags"), protocol::AMM_TWO_ASSET_IF_EMPTY_FLAG);
        });
        let before = view.entries.len();
        assert_eq!(
            run_dex_read_view_preclaim(&view, &nonempty_deposit, TxType::AMM_DEPOSIT),
            Some(Ter::TEC_AMM_NOT_EMPTY),
            "pool state precedes all later Deposit checks"
        );
        assert_eq!(view.entries.len(), before, "preclaim is read-only");

        let mut empty_view = View::default();
        empty_view.insert(amm_entry(account(24), asset, asset2, 0));
        assert_eq!(
            run_dex_read_view_preclaim(&empty_view, &missing, TxType::AMM_WITHDRAW),
            Some(Ter::TEC_AMM_EMPTY),
            "pool state precedes all later Withdraw checks"
        );
    }

    #[test]
    fn amm_clawback_reports_missing_issuer_before_other_facts() {
        let issuer = account(7);
        let holder = account(8);
        let asset = Asset::from(issue(9, issuer));
        let asset2 = Asset::from(issue(10, issuer));
        let tx = STTx::new(TxType::AMM_CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_issue(
                sf("sfAsset"),
                protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
            );
        });

        assert_eq!(
            run_dex_read_view_preclaim(&View::default(), &tx, TxType::AMM_CLAWBACK),
            Some(protocol::Ter::TER_NO_ACCOUNT)
        );
    }

    #[test]
    fn amm_clawback_legacy_permission_precedes_mpt_issuance_read() {
        let issuer = account(7);
        let holder = account(8);
        let amm_account = account(9);
        let asset = Asset::MPTIssue(protocol::MPTIssue::new(protocol::make_mpt_id(1, issuer)));
        let asset2 = Asset::from(issue(10, issuer));
        let mut view = View::default();
        view.insert(account_entry(issuer, 1));
        view.insert(account_entry(holder, 1));
        view.insert(amm_entry(amm_account, asset, asset2, 10));
        let Asset::MPTIssue(mpt) = asset else {
            unreachable!("fixture uses MPT")
        };
        view.fail_reads
            .insert(protocol::mpt_issuance_keylet_from_mptid(mpt.mpt_id()).key);
        let tx = STTx::new(TxType::AMM_CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_issue(
                sf("sfAsset"),
                protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                protocol::STIssue::new_with_asset(sf("sfAsset2"), asset2),
            );
        });

        assert_eq!(
            run_dex_read_view_preclaim(&view, &tx, TxType::AMM_CLAWBACK),
            Some(Ter::TEC_NO_PERMISSION),
            "legacy issuer permission must precede checkClawAsset's MPT issuance read"
        );
    }
}
