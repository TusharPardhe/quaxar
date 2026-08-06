//! Immutable `ReadView` typed preclaim helpers for the DEX transaction family.
//!
//! These helpers are deliberately limited to transaction types whose complete
//! `preclaim(...)` paths can be evaluated from an immutable ledger view. They
//! do not call apply code, create sandboxes, or return a permissive result for
//! a transaction type they do not own.

use basics::base_uint::Uint160;
use ledger::ReadView;
use protocol::{
    AccountID, ApplyFlags, Asset, STAmount, STTx, Ter, TxType, XRPAmount, get_field_by_symbol,
    lsfAllowTrustLineClawback, lsfDefaultRipple, lsfGlobalFreeze, lsfHighAuth, lsfHighDeepFreeze,
    lsfHighFreeze, lsfLowAuth, lsfLowDeepFreeze, lsfLowFreeze, lsfMPTAuthorized, lsfMPTCanClawback,
    lsfMPTRequireAuth, lsfNoFreeze, lsfRequireAuth,
};

use crate::{
    AMMClawbackPreclaimFacts, AMMCreatePreclaimFacts, AMMDeletePreclaimFacts, AMMVotePreclaimFacts,
    AmmBidPreclaimFacts, AmmBidSlotPricePreclaimFacts, OfferCancelPreclaimFacts,
    run_amm_bid_preclaim, run_amm_clawback_preclaim_facts, run_amm_create_preclaim_facts,
    run_amm_delete_preclaim_facts, run_amm_vote_preclaim_facts, run_offer_cancel_preclaim,
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
    let owner_count = account_sle
        .get_field_u32(sf("sfOwnerCount"))
        .saturating_add(owner_count_adjustment) as usize;
    let reserve = view.fees().account_reserve(owner_count) as i64;
    Ok(XRPAmount::from_drops(
        account_sle
            .get_field_amount(sf("sfBalance"))
            .xrp()
            .drops()
            .saturating_sub(reserve)
            .max(0),
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
            let individually_frozen = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
                .is_some_and(|line| {
                    line.get_field_u32(sf("sfFlags"))
                        & if account > issue.account {
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
            let maximum = issuance
                .is_field_present(sf("sfMaximumAmount"))
                .then(|| issuance.get_field_u64(sf("sfMaximumAmount")))
                .unwrap_or(i64::MAX as u64);
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
            let Some(line) = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
            else {
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

fn preclaim_offer_create<V: ReadView>(
    view: &V,
    tx: &STTx,
    apply_flags: ApplyFlags,
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

    if tx.is_field_present(sf("sfOfferSequence"))
        && account_sle.get_field_u32(sf("sfSequence")) <= tx.get_field_u32(sf("sfOfferSequence"))
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
    let amm_keylet = protocol::keylet::amm(amount.asset(), amount2.asset());
    let single_asset_vault_enabled = view
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"));
    let address_collision = single_asset_vault_enabled
        && ledger::pseudo_account_address(view, amm_keylet.key).is_zero();

    Ok(run_amm_create_preclaim_facts(AMMCreatePreclaimFacts {
        amm_exists: view.read(amm_keylet).map_err(|_| read_error())?.is_some(),
        amount_auth_result: asset_auth(view, account, amount.asset())?,
        amount2_auth_result: asset_auth(view, account, amount2.asset())?,
        amount_frozen_result: asset_frozen(view, account, amount.asset())?,
        amount2_frozen_result: asset_frozen(view, account, amount2.asset())?,
        amount_no_default_ripple: no_default_ripple(view, amount.asset())?,
        amount2_no_default_ripple: no_default_ripple(view, amount2.asset())?,
        xrp_reserve_positive: xrp_liquid(view, account, 1)?.drops() > 0,
        amount_insufficient_balance: asset_funds(view, account, &amount, true)? < amount,
        amount2_insufficient_balance: asset_funds(view, account, &amount2, true)? < amount2,
        amount_is_lp_token: is_lp_token(view, amount.asset())?,
        amount2_is_lp_token: is_lp_token(view, amount2.asset())?,
        address_collision,
        amount_mpt_trade_transfer_result: ledger::mptoken_helpers::can_mpt_trade_and_transfer(
            view,
            &amount.asset(),
            &account,
            &account,
        )
        .map_err(|_| read_error())?,
        amount2_mpt_trade_transfer_result: ledger::mptoken_helpers::can_mpt_trade_and_transfer(
            view,
            &amount2.asset(),
            &account,
            &account,
        )
        .map_err(|_| read_error())?,
        amm_clawback_enabled: view.rules().enabled(&protocol::feature_id("AMMClawback")),
        amount_clawback_disabled_result: clawback_disabled(view, amount.asset())?,
        amount2_clawback_disabled_result: clawback_disabled(view, amount2.asset())?,
        // Current `AMMCreate::preclaim` has no vault-share exclusion. Keep
        // that explicitly false so the fact adapter cannot invent a rejection
        // absent from the matched rippled source.
        amount_is_vault_share: false,
        amount2_is_vault_share: false,
        single_asset_vault_enabled,
    }))
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
    let account = tx.get_account_id(sf("sfAccount"));
    let lp_holds = lp_holds(view, &amm, account)?;
    Ok(run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
        amm_exists: true,
        lp_token_balance_signum: amm.get_field_amount(sf("sfLPTokenBalance")).signum(),
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

    let account = tx.get_account_id(sf("sfAccount"));
    let lp_tokens = lp_holds(view, &amm, account)?;
    let lp_total = amm.get_field_amount(sf("sfLPTokenBalance"));
    let auth_accounts = if tx.is_field_present(sf("sfAuthAccounts")) {
        tx.get_field_array(sf("sfAuthAccounts"))
            .iter()
            .map(|entry| {
                read_account(view, entry.get_account_id(sf("sfAccount"))).map(|sle| sle.is_some())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
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
    let claw_two_assets = (tx.get_flags() & protocol::AMM_CLAWBACK_TWO_ASSETS_FLAG) != 0;
    Ok(run_amm_clawback_preclaim_facts(AMMClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        amm_exists: true,
        mptokens_v2_enabled: view.rules().enabled(&protocol::feature_id("MPTokensV2")),
        issuer_allows_trustline_clawback: (issuer_flags & lsfAllowTrustLineClawback) != 0,
        issuer_no_freeze: (issuer_flags & lsfNoFreeze) != 0,
        asset_claw_allowed: clawback_asset_allowed(view, issuer, issuer_flags, asset)?,
        claw_two_assets,
        asset2_claw_allowed: clawback_asset_allowed(view, issuer, issuer_flags, asset2)?,
    }))
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
        TxType::OFFER_CREATE => preclaim_offer_create(view, tx, apply_flags),
        TxType::OFFER_CANCEL => preclaim_offer_cancel(view, tx),
        TxType::AMM_CREATE => preclaim_amm_create(view, tx),
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
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        AccountID, ApplyFlags, Asset, Currency, IOUAmount, Issue, LedgerEntryType, STAmount,
        STLedgerEntry, STTx, Ter, TxType, XRPAmount, get_field_by_symbol,
    };

    use super::{run_dex_read_view_preclaim, run_dex_read_view_preclaim_with_flags};

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
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
            Rules::default()
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

    fn amm_entry(amm_account: AccountID, asset: Asset, asset2: Asset, lp: i64) -> STLedgerEntry {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AMM,
            protocol::keylet::amm(asset, asset2).key,
        );
        entry.set_account_id(sf("sfAccount"), amm_account);
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
}
