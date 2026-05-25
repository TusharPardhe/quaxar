//! Full reference AMMBid::applyBid parity implementation.
//!
//! Ports: the reference source applyBid(), redeemIOU(), accountSend() for LP tokens.

use std::sync::Arc;

use basics::number::{self, NumberParts as RuntimeNumber};
use ledger::ApplyView;
use protocol::{
    AUCTION_SLOT_DISCOUNTED_FEE_FRACTION, AUCTION_SLOT_MIN_FEE_FRACTION,
    AUCTION_SLOT_TIME_INTERVALS, AccountID, Asset, Issue, STAmount, STLedgerEntry, STObject,
    TOTAL_TIME_SLOT_SECS, Ter, get_field_by_symbol as sf,
};

const TAILING_SLOT: u8 = AUCTION_SLOT_TIME_INTERVALS as u8 - 1;

/// Read LP token balance for an account from the trust line.
fn read_lp_balance<V: ApplyView>(view: &mut V, account: AccountID, lp_issue: Issue) -> STAmount {
    let line_keylet = protocol::line(account, lp_issue.account, lp_issue.currency);
    let Some(state) = view.peek(line_keylet).ok().flatten() else {
        return STAmount::default();
    };
    let mut balance = state.get_field_amount(sf("sfBalance"));
    if account > lp_issue.account {
        balance.negate();
    }
    if balance.signum() > 0 {
        balance
    } else {
        STAmount::default()
    }
}

fn redeem_iou<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    let b_sender_high = *account > issue.account;
    let line_keylet = protocol::line(*account, issue.account, issue.currency);
    let Some(state) = view.peek(line_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };
    let mut final_balance = state.get_field_amount(sf("sfBalance"));
    if b_sender_high {
        final_balance.negate();
    }
    final_balance -= amount.clone();
    if b_sender_high {
        final_balance.negate();
    }
    let mut obj = state.clone_as_object();
    obj.set_field_amount(sf("sfBalance"), final_balance);
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *state.key())));
    Ter::TES_SUCCESS
}

fn issue_iou<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    let b_sender_high = *account > issue.account;
    let line_keylet = protocol::line(*account, issue.account, issue.currency);
    let Some(state) = view.peek(line_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };
    let mut final_balance = state.get_field_amount(sf("sfBalance"));
    if b_sender_high {
        final_balance.negate();
    }
    final_balance += amount.clone();
    if b_sender_high {
        final_balance.negate();
    }
    let mut obj = state.clone_as_object();
    obj.set_field_amount(sf("sfBalance"), final_balance);
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *state.key())));
    Ter::TES_SUCCESS
}

fn account_send_lp<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    if amount.signum() <= 0 {
        return Ter::TES_SUCCESS;
    }
    let res = redeem_iou(view, from, amount, issue);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    issue_iou(view, to, amount, issue)
}

fn number_from_i64(v: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(v, 0, basics::number::get_mantissa_scale())
        .unwrap_or_else(|_| {
            let iou = protocol::IOUAmount::from_parts(v, 0).unwrap_or_default();
            RuntimeNumber::from(iou)
        })
}

fn number_from_parts(mantissa: i64, exponent: i32) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(mantissa, exponent, basics::number::get_mantissa_scale())
        .unwrap_or_else(|_| {
            let iou = protocol::IOUAmount::from_parts(mantissa, exponent).unwrap_or_default();
            RuntimeNumber::from(iou)
        })
}

pub fn apply_amm_bid<V: ApplyView>(view: &mut V, sttx: &protocol::STTx) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));

    // Get AMM SLE
    let asset1_issue = sttx.get_field_amount(sf("sfAsset")).issue();
    let asset2_issue = sttx.get_field_amount(sf("sfAsset2")).issue();
    let amm_keylet = protocol::amm(Asset::Issue(asset1_issue), Asset::Issue(asset2_issue));
    let Some(amm_sle) = view.peek(amm_keylet).ok().flatten() else {
        return Ter::TEC_INTERNAL;
    };

    let lpt_amm_balance = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
    let lp_issue = lpt_amm_balance.issue();

    // Get account's LP token holdings from trust line
    let lp_tokens = read_lp_balance(view, account, lp_issue);
    if lp_tokens.signum() <= 0 {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    let current = view.parent_close_time().as_seconds() as u64;

    // Compute discounted fee and min slot price
    let trading_fee = amm_sle.get_field_u16(sf("sfTradingFee"));
    let discounted_fee = trading_fee / AUCTION_SLOT_DISCOUNTED_FEE_FRACTION as u16;
    let fee_number = protocol::get_fee(trading_fee);
    let lpt_balance_number = ledger::amm_helpers::stamount_as_number(&lpt_amm_balance);
    let min_fee_frac = number_from_i64(AUCTION_SLOT_MIN_FEE_FRACTION as i64);
    let min_slot_price = lpt_balance_number * fee_number / min_fee_frac;

    // Get auction slot
    let auction_slot_obj = if amm_sle.is_field_present(sf("sfAuctionSlot")) {
        amm_sle.get_field_object(sf("sfAuctionSlot"))
    } else {
        STObject::new(sf("sfAuctionSlot"))
    };

    let time_slot = protocol::amm_auction_time_slot(current, &auction_slot_obj);

    // Check if current owner is valid
    let has_valid_owner = if let Some(slot) = time_slot {
        if slot < TAILING_SLOT {
            let slot_account = auction_slot_obj.get_account_id(sf("sfAccount"));
            let acct_keylet = protocol::account_keylet(basics::base_uint::Uint160::from_void(
                slot_account.data(),
            ));
            view.peek(acct_keylet).ok().flatten().is_some()
        } else {
            false
        }
    } else {
        false
    };

    // Get bidMin/bidMax
    let bid_min = if sttx.is_field_present(sf("sfBidMin")) {
        Some(sttx.get_field_amount(sf("sfBidMin")))
    } else {
        None
    };
    let bid_max = if sttx.is_field_present(sf("sfBidMax")) {
        Some(sttx.get_field_amount(sf("sfBidMax")))
    } else {
        None
    };

    let lp_tokens_number = ledger::amm_helpers::stamount_as_number(&lp_tokens);

    // Compute pay price based on whether slot is owned
    let (pay_price, refund_amount) = if !has_valid_owner {
        match get_pay_price(min_slot_price, &bid_min, &bid_max, lp_tokens_number) {
            Ok(price) => (price, number_from_i64(0)),
            Err(ter) => return ter,
        }
    } else {
        let price_purchased = ledger::amm_helpers::stamount_as_number(
            &auction_slot_obj.get_field_amount(sf("sfPrice")),
        );
        let slot = time_slot.unwrap() as i64;
        let intervals = number_from_i64(AUCTION_SLOT_TIME_INTERVALS as i64);
        let fraction_used = number_from_i64(slot + 1) / intervals;
        let one = number_from_i64(1);
        let fraction_remaining = one - fraction_used;
        let p105 = number_from_parts(105, -2);

        let computed = if slot == 0 {
            price_purchased * p105 + min_slot_price
        } else {
            let decay = one - number::power(fraction_used, 60).unwrap_or(number_from_i64(0));
            price_purchased * p105 * decay + min_slot_price
        };

        let refund = fraction_remaining * price_purchased;

        match get_pay_price(computed, &bid_min, &bid_max, lp_tokens_number) {
            Ok(price) => {
                if refund > price {
                    return Ter::TEC_INTERNAL;
                }
                (price, refund)
            }
            Err(ter) => return ter,
        }
    };

    // Refund previous owner if needed
    let zero = number_from_i64(0);
    if refund_amount > zero && has_valid_owner {
        let prev_owner = auction_slot_obj.get_account_id(sf("sfAccount"));
        let refund_st = ledger::amm_helpers::to_st_amount(lp_issue, refund_amount);
        let res = account_send_lp(view, &account, &prev_owner, &refund_st, &lp_issue);
        if res != Ter::TES_SUCCESS {
            return res;
        }
    }

    // Burn LP tokens (pay_price - refund)
    let burn_amount = pay_price - refund_amount;
    let burn_st = ledger::amm_helpers::to_st_amount(lp_issue, burn_amount);
    let adjusted_burn = ledger::amm_helpers::adjust_lp_tokens(
        &lpt_amm_balance,
        &burn_st,
        ledger::amm_helpers::IsDeposit::No,
    );

    if adjusted_burn >= lpt_amm_balance {
        return Ter::TEC_INTERNAL;
    }

    // Redeem (burn) LP tokens from bidder
    let res = redeem_iou(view, &account, &adjusted_burn, &lp_issue);
    if res != Ter::TES_SUCCESS {
        return res;
    }

    // Update AMM object
    let new_lpt_balance = lpt_amm_balance - adjusted_burn;
    let mut amm_obj = amm_sle.clone_as_object();

    // Update auction slot
    let mut slot = STObject::new(sf("sfAuctionSlot"));
    slot.set_account_id(sf("sfAccount"), account);
    slot.set_field_u32(sf("sfExpiration"), current as u32 + TOTAL_TIME_SLOT_SECS);
    if discounted_fee != 0 {
        slot.set_field_u16(sf("sfDiscountedFee"), discounted_fee);
    }
    slot.set_field_amount(
        sf("sfPrice"),
        ledger::amm_helpers::to_st_amount(lp_issue, pay_price),
    );
    if sttx.is_field_present(sf("sfAuthAccounts")) {
        slot.set_field_array(
            sf("sfAuthAccounts"),
            sttx.get_field_array(sf("sfAuthAccounts")),
        );
    }
    amm_obj.set_field_object(sf("sfAuctionSlot"), slot);
    amm_obj.set_field_amount(sf("sfLPTokenBalance"), new_lpt_balance);
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        amm_obj,
        *amm_sle.key(),
    )));

    Ter::TES_SUCCESS
}

/// Compute the actual price to pay, respecting bidMin/bidMax constraints.
fn get_pay_price(
    computed_price: RuntimeNumber,
    bid_min: &Option<STAmount>,
    bid_max: &Option<STAmount>,
    lp_tokens: RuntimeNumber,
) -> Result<RuntimeNumber, Ter> {
    let pay_price = match (bid_min, bid_max) {
        (Some(min), Some(max)) => {
            let min_n = ledger::amm_helpers::stamount_as_number(min);
            let max_n = ledger::amm_helpers::stamount_as_number(max);
            if computed_price <= max_n {
                if computed_price > min_n {
                    computed_price
                } else {
                    min_n
                }
            } else {
                return Err(Ter::TEC_AMM_FAILED);
            }
        }
        (Some(min), None) => {
            let min_n = ledger::amm_helpers::stamount_as_number(min);
            if computed_price > min_n {
                computed_price
            } else {
                min_n
            }
        }
        (None, Some(max)) => {
            let max_n = ledger::amm_helpers::stamount_as_number(max);
            if computed_price <= max_n {
                computed_price
            } else {
                return Err(Ter::TEC_AMM_FAILED);
            }
        }
        (None, None) => computed_price,
    };
    if pay_price > lp_tokens {
        return Err(Ter::TEC_AMM_INVALID_TOKENS);
    }
    Ok(pay_price)
}

/// Public wrapper for redeem_iou (used by AMM_WITHDRAW)
pub fn redeem_iou_pub<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    redeem_iou(view, account, amount, issue)
}

/// Public wrapper for issue_iou (used by AMM_DEPOSIT)
pub fn issue_iou_pub<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    issue_iou(view, account, amount, issue)
}
