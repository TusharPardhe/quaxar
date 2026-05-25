//! `xrpl/ledger/helpers/AMMUtils.*` compatibility-safe read helpers.

use basics::base_uint::Uint160;
use basics::expected::{Expected, Unexpected};
use protocol::{
    AccountID, Currency, Issue, STAmount, STIssue, STLedgerEntry, Ter, amm_lpt_currency,
    get_field_by_symbol, invalid_amm_asset_pair, is_xrp_currency, line, owner_dir_keylet,
    page_keylet, sf_generic,
};
use shamap::traversal::TraversalError;

use crate::{FreezeHandling, Ledger, account_funds, is_frozen};

fn issue_from_stissue(issue: STIssue) -> Option<Issue> {
    match issue.asset() {
        protocol::Asset::Issue(issue) => Some(issue),
        protocol::Asset::MPTIssue(_) => None,
    }
}

fn zero_issue_amount(issue: Issue) -> STAmount {
    STAmount::new_with_asset(sf_generic(), issue, 0, 0, false)
}

fn raw_account_id(id: AccountID) -> Uint160 {
    Uint160::from_slice(id.data()).expect("account width")
}

pub fn amm_pool_holds(
    view: &Ledger,
    amm_account_id: AccountID,
    issue1: Issue,
    issue2: Issue,
    freeze_handling: FreezeHandling,
) -> Result<(STAmount, STAmount), TraversalError> {
    let asset_in_balance = account_funds(
        view,
        amm_account_id,
        &zero_issue_amount(issue1),
        freeze_handling,
    )?;
    let asset_out_balance = account_funds(
        view,
        amm_account_id,
        &zero_issue_amount(issue2),
        freeze_handling,
    )?;
    Ok((asset_in_balance, asset_out_balance))
}

pub fn amm_holds(
    view: &Ledger,
    amm_sle: &STLedgerEntry,
    opt_issue1: Option<Issue>,
    opt_issue2: Option<Issue>,
    freeze_handling: FreezeHandling,
) -> Expected<(STAmount, STAmount, STAmount), Ter> {
    let Some(issue1) = issue_from_stissue(amm_sle.get_field_issue(get_field_by_symbol("sfAsset")))
    else {
        return Unexpected::new(Ter::TEC_AMM_INVALID_TOKENS).into();
    };
    let Some(issue2) = issue_from_stissue(amm_sle.get_field_issue(get_field_by_symbol("sfAsset2")))
    else {
        return Unexpected::new(Ter::TEC_AMM_INVALID_TOKENS).into();
    };

    let issues = if let (Some(opt1), Some(opt2)) = (opt_issue1, opt_issue2) {
        if invalid_amm_asset_pair(opt1, opt2, Some((issue1, issue2))) != Ter::TES_SUCCESS {
            return Unexpected::new(Ter::TEC_AMM_INVALID_TOKENS).into();
        }
        Some((opt1, opt2))
    } else if let Some(check_issue) = opt_issue1.or(opt_issue2) {
        if check_issue == issue1 {
            Some((issue1, issue2))
        } else if check_issue == issue2 {
            Some((issue2, issue1))
        } else {
            None
        }
    } else {
        Some((issue1, issue2))
    };

    let Some((first_issue, second_issue)) = issues else {
        return Unexpected::new(Ter::TEC_AMM_INVALID_TOKENS).into();
    };

    let Ok((asset1, asset2)) = amm_pool_holds(
        view,
        amm_sle.get_account_id(get_field_by_symbol("sfAccount")),
        first_issue,
        second_issue,
        freeze_handling,
    ) else {
        return Unexpected::new(Ter::TEC_INTERNAL).into();
    };

    Expected::from_value((
        asset1,
        asset2,
        amm_sle.get_field_amount(get_field_by_symbol("sfLPTokenBalance")),
    ))
}

pub fn amm_lp_holds(
    view: &Ledger,
    cur1: Currency,
    cur2: Currency,
    amm_account: AccountID,
    lp_account: AccountID,
) -> Result<STAmount, TraversalError> {
    let currency = amm_lpt_currency(cur1, cur2);
    let issue = Issue::new(currency, amm_account);
    let Some(sle) = view.read(line(lp_account, amm_account, currency))? else {
        return Ok(zero_issue_amount(issue));
    };
    if is_frozen(
        view,
        raw_account_id(lp_account),
        currency,
        raw_account_id(amm_account),
    )? {
        return Ok(zero_issue_amount(issue));
    }

    let mut amount = sle.get_field_amount(get_field_by_symbol("sfBalance"));
    if lp_account > amm_account {
        amount.negate();
    }
    amount.set_issuer(amm_account);
    Ok(amount)
}

pub fn amm_lp_holds_from_sle(
    view: &Ledger,
    amm_sle: &STLedgerEntry,
    lp_account: AccountID,
) -> Result<STAmount, TraversalError> {
    let Some(issue1) = issue_from_stissue(amm_sle.get_field_issue(get_field_by_symbol("sfAsset")))
    else {
        return Ok(STAmount::default());
    };
    let Some(issue2) = issue_from_stissue(amm_sle.get_field_issue(get_field_by_symbol("sfAsset2")))
    else {
        return Ok(STAmount::default());
    };
    amm_lp_holds(
        view,
        issue1.currency,
        issue2.currency,
        amm_sle.get_account_id(get_field_by_symbol("sfAccount")),
        lp_account,
    )
}

pub fn get_trading_fee(view: &Ledger, amm_sle: &STLedgerEntry, account: AccountID) -> u16 {
    if amm_sle.is_field_present(get_field_by_symbol("sfAuctionSlot")) {
        let auction_slot = amm_sle.get_field_object(get_field_by_symbol("sfAuctionSlot"));
        let expiration = u64::from(auction_slot.get_field_u32(get_field_by_symbol("sfExpiration")));
        if u64::from(view.header().parent_close_time) < expiration {
            if auction_slot.get_account_id(get_field_by_symbol("sfAccount")) == account {
                return auction_slot.get_field_u16(get_field_by_symbol("sfDiscountedFee"));
            }
            if auction_slot.is_field_present(get_field_by_symbol("sfAuthAccounts")) {
                for acct in auction_slot
                    .get_field_array(get_field_by_symbol("sfAuthAccounts"))
                    .iter()
                {
                    if acct.get_account_id(get_field_by_symbol("sfAccount")) == account {
                        return auction_slot.get_field_u16(get_field_by_symbol("sfDiscountedFee"));
                    }
                }
            }
        }
    }
    amm_sle.get_field_u16(get_field_by_symbol("sfTradingFee"))
}

pub fn amm_account_holds(
    view: &Ledger,
    amm_account_id: AccountID,
    issue: Issue,
) -> Result<STAmount, TraversalError> {
    if is_xrp_currency(issue.currency) {
        if let Some(sle) = view.read(protocol::account_keylet(raw_account_id(amm_account_id)))? {
            return Ok(sle.get_field_amount(get_field_by_symbol("sfBalance")));
        }
    } else if let Some(sle) = view.read(line(amm_account_id, issue.account, issue.currency))?
        && !is_frozen(
            view,
            raw_account_id(amm_account_id),
            issue.currency,
            raw_account_id(issue.account),
        )?
    {
        let mut amount = sle.get_field_amount(get_field_by_symbol("sfBalance"));
        if amm_account_id > issue.account {
            amount.negate();
        }
        amount.set_issuer(issue.account);
        return Ok(amount);
    }

    Ok(zero_issue_amount(issue))
}

pub fn is_only_liquidity_provider(
    view: &Ledger,
    amm_issue: Issue,
    lp_account: AccountID,
) -> Expected<bool, Ter> {
    let mut n_lp_token_trust_lines = 0u8;
    let mut n_iou_trust_lines = 0u8;
    let mut has_amm = false;
    let root = owner_dir_keylet(raw_account_id(amm_issue.account));
    let mut current_index = root;
    let mut limit = 10u8;

    while limit > 0 {
        limit -= 1;
        let owner_dir = match view.read(current_index) {
            Ok(owner_dir) => owner_dir,
            Err(_) => return Unexpected::new(Ter::TEC_INTERNAL).into(),
        };
        let Some(owner_dir) = owner_dir else {
            return Unexpected::new(Ter::TEC_INTERNAL).into();
        };
        for key in owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .iter()
            .copied()
        {
            let sle = match view.read(protocol::child_keylet(key)) {
                Ok(sle) => sle,
                Err(_) => return Unexpected::new(Ter::TEC_INTERNAL).into(),
            };
            let Some(sle) = sle else {
                return Unexpected::new(Ter::TEC_INTERNAL).into();
            };
            if sle.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
                == protocol::LedgerEntryType::AMM as u16
            {
                if has_amm {
                    return Unexpected::new(Ter::TEC_INTERNAL).into();
                }
                has_amm = true;
                continue;
            }
            if sle.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
                != protocol::LedgerEntryType::RippleState as u16
            {
                return Unexpected::new(Ter::TEC_INTERNAL).into();
            }
            let low_limit = sle.get_field_amount(get_field_by_symbol("sfLowLimit"));
            let high_limit = sle.get_field_amount(get_field_by_symbol("sfHighLimit"));
            let is_lp_trustline = low_limit.issue().issuer() == lp_account
                || high_limit.issue().issuer() == lp_account;
            let is_lp_token_trustline =
                low_limit.issue() == amm_issue || high_limit.issue() == amm_issue;

            if is_lp_trustline {
                if is_lp_token_trustline {
                    n_lp_token_trust_lines = n_lp_token_trust_lines.saturating_add(1);
                    if n_lp_token_trust_lines > 1 {
                        return Unexpected::new(Ter::TEC_INTERNAL).into();
                    }
                } else {
                    n_iou_trust_lines = n_iou_trust_lines.saturating_add(1);
                    if n_iou_trust_lines > 2 {
                        return Unexpected::new(Ter::TEC_INTERNAL).into();
                    }
                }
            } else if is_lp_token_trustline {
                return Expected::from_value(false);
            } else {
                n_iou_trust_lines = n_iou_trust_lines.saturating_add(1);
                if n_iou_trust_lines > 2 {
                    return Unexpected::new(Ter::TEC_INTERNAL).into();
                }
            }
        }

        let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if next == 0 {
            if n_lp_token_trust_lines != 1 || n_iou_trust_lines == 0 || n_iou_trust_lines > 2 {
                return Unexpected::new(Ter::TEC_INTERNAL).into();
            }
            return Expected::from_value(true);
        }
        current_index = page_keylet(root, next);
    }

    Unexpected::new(Ter::TEC_INTERNAL).into()
}
