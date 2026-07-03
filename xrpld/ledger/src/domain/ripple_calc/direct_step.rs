//! Full reference the reference source parity.
//!
//! DirectStep handles IOU transfer between two accounts on the same trust line.
//! It computes max flow, applies quality adjustments, and executes the transfer.

use crate::ApplyView;
use crate::domain::ripple_state_helpers;
use protocol::{AccountID, Currency, IOUAmount, Issue, STAmount, Ter, get_field_by_symbol as sf};

/// Debt direction: is the source redeeming (paying back) or issuing (lending)?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebtDirection {
    /// Source is redeeming (balance > 0 from source perspective)
    Redeems,
    /// Source is issuing (balance <= 0 from source perspective)
    Issues,
}

/// Quality direction for rate lookup
#[derive(Debug, Clone, Copy)]
pub enum QualityDirection {
    In,
    Out,
}

/// Cache for a DirectStep computation
#[derive(Debug, Clone)]
pub struct DirectStepCache {
    pub input: IOUAmount,
    pub src_to_dst: IOUAmount,
    pub output: IOUAmount,
    pub src_debt_dir: DebtDirection,
}

/// Compute the maximum payment flow on a trust line from src to dst.
/// Returns (max_flow, debt_direction).
pub fn max_payment_flow<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    currency: Currency,
) -> (IOUAmount, DebtDirection) {
    // XRP has no trust line — DirectStep only handles IOU.
    if protocol::is_xrp_currency(currency) {
        return (IOUAmount::new(), DebtDirection::Issues);
    }

    // Compute how much src can send to dst on this trust line.
    // Matches C++ maxPaymentFlow:
    //   srcOwed = accountHolds(sb, src_, currency_, dst_)
    //   if srcOwed > 0: return (srcOwed, Redeems)
    //   else: return (creditLimit2(sb, dst_, src_, currency_) + srcOwed, Issues)
    //
    // credit_balance(view, src, dst, currency) returns how much src holds
    // (from src's perspective). Positive = dst owes src (src can redeem).
    let balance = ripple_state_helpers::credit_balance(view, src, dst, currency);

    if balance.signum() > 0 {
        // Source is redeeming (has positive balance = dst owes src)
        if balance.native() {
            return (IOUAmount::new(), DebtDirection::Issues);
        }
        (balance.iou(), DebtDirection::Redeems)
    } else {
        // Source is issuing (has zero or negative balance)
        // Max flow is dst's credit limit for src (how much dst allows src to owe)
        // reference: creditLimit2(sb, dst_, src_, currency_) + srcOwed
        let line_keylet = protocol::line(*src, *dst, currency);
        let limit = if let Some(state) = view.peek(line_keylet).ok().flatten() {
            let b_src_high = *src > *dst;
            // We need DST's limit — which is the opposite side from src
            let limit_field = if b_src_high {
                sf("sfLowLimit") // src is high, so dst is low → dst's limit is sfLowLimit
            } else {
                sf("sfHighLimit") // src is low, so dst is high → dst's limit is sfHighLimit
            };
            let limit_amount = state.get_field_amount(limit_field);
            if limit_amount.native() {
                IOUAmount::new()
            } else {
                limit_amount.iou()
            }
        } else {
            IOUAmount::new()
        };
        // Available = limit + srcOwed (srcOwed is negative, so this subtracts)
        let src_owed = if balance.native() {
            IOUAmount::new()
        } else {
            balance.iou()
        };
        let available = limit
            .checked_add(src_owed)
            .unwrap_or_else(|_| IOUAmount::new());
        if available.signum() <= 0 {
            (IOUAmount::new(), DebtDirection::Issues)
        } else {
            (available, DebtDirection::Issues)
        }
    }
}

/// Get the quality (transfer rate) for a direction on a trust line.
/// Returns the quality as a u32 (QUALITY_ONE = 1000000000 = no adjustment).
pub fn get_quality<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    currency: Currency,
    direction: QualityDirection,
) -> u32 {
    let line_keylet = protocol::line(*src, *dst, currency);
    let Some(state) = view.peek(line_keylet).ok().flatten() else {
        return protocol::QUALITY_ONE;
    };
    let b_src_high = *src > *dst;
    let field = match direction {
        QualityDirection::Out => {
            if b_src_high {
                sf("sfHighQualityOut")
            } else {
                sf("sfLowQualityOut")
            }
        }
        QualityDirection::In => {
            if b_src_high {
                sf("sfHighQualityIn")
            } else {
                sf("sfLowQualityIn")
            }
        }
    };
    let q = state.get_field_u32(field);
    if q == 0 { protocol::QUALITY_ONE } else { q }
}

/// Execute a DirectStep: transfer IOU from src to dst on their shared trust line.
/// This is the core of what DirectStep::fwd/rev does.
///
/// Returns (amount_in, amount_out) — how much was consumed and produced.
pub fn execute_direct_step<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    currency: Currency,
    max_input: IOUAmount,
    _debt_direction: DebtDirection,
) -> Result<(IOUAmount, IOUAmount), Ter> {
    if max_input.is_zero() {
        return Ok((IOUAmount::new(), IOUAmount::new()));
    }

    // Get max flow and qualities
    let (max_src_to_dst, src_debt_dir) = max_payment_flow(view, src, dst, currency);
    if max_src_to_dst.is_zero() {
        return Ok((IOUAmount::new(), IOUAmount::new()));
    }

    let (src_q_out, dst_q_in) = if src_debt_dir == DebtDirection::Redeems {
        qualities_src_redeems(view, src, dst, currency)
    } else {
        // reference: prevStepDebtDirection defaults to Issues when no previous step
        qualities_src_issues(view, src, dst, currency, false)
    };

    let src_to_dst = crate::domain::mul_ratio::mul_ratio(
        max_input,
        crate::domain::mul_ratio::QUALITY_ONE,
        src_q_out,
        false,
    );

    let (actual_in, actual_out, actual_src_to_dst) = if src_to_dst <= max_src_to_dst {
        // Non-limiting
        let out = crate::domain::mul_ratio::mul_ratio(
            src_to_dst,
            dst_q_in,
            crate::domain::mul_ratio::QUALITY_ONE,
            false,
        );
        (max_input, out, src_to_dst)
    } else {
        // Limiting — use max available
        let actual_in = crate::domain::mul_ratio::mul_ratio(
            max_src_to_dst,
            src_q_out,
            crate::domain::mul_ratio::QUALITY_ONE,
            true,
        );
        let out = crate::domain::mul_ratio::mul_ratio(
            max_src_to_dst,
            dst_q_in,
            crate::domain::mul_ratio::QUALITY_ONE,
            false,
        );
        (actual_in, out, max_src_to_dst)
    };

    if actual_src_to_dst.is_zero() {
        return Ok((IOUAmount::new(), IOUAmount::new()));
    }

    // Execute the transfer
    let issue = Issue {
        currency,
        account: if src_debt_dir == DebtDirection::Redeems {
            *dst
        } else {
            *src
        },
    };
    let amount = STAmount::from_iou_amount(sf("sfAmount"), actual_src_to_dst, issue);
    let res = ripple_state_helpers::ripple_credit(view, src, dst, &amount, true);
    if res != Ter::TES_SUCCESS {
        return Err(res);
    }

    Ok((actual_in, actual_out))
}

/// Compute qualities for when source redeems (positive balance).
/// Returns (src_quality_out, dst_quality_in).
pub fn qualities_src_redeems<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    currency: Currency,
) -> (u32, u32) {
    let src_q_out = get_quality(view, src, dst, currency, QualityDirection::Out);
    let dst_q_in = get_quality(view, dst, src, currency, QualityDirection::In);
    (src_q_out, dst_q_in)
}

/// Compute qualities for when source issues (zero/negative balance).
/// Returns (src_quality_out, dst_quality_in).
pub fn qualities_src_issues<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    currency: Currency,
    prev_step_redeems: bool,
) -> (u32, u32) {
    let src_q_out = if prev_step_redeems {
        ripple_state_helpers::transfer_rate(view, src)
    } else {
        protocol::QUALITY_ONE
    };
    let dst_q_in = get_quality(view, dst, src, currency, QualityDirection::In);
    (src_q_out, dst_q_in)
}
