//! Step execution matching reference DirectStep, BookStep, XRPEndpointStep.

use super::StepKind;
use crate::ApplyView;
use crate::domain::ripple_state_helpers;
use protocol::{
    AccountID, Issue, Quality, STAmount, Ter, XRPAmount, get_field_by_symbol as sf, xrp_account,
};

/// Execute a single step forward: given input, produce output.
pub fn execute_step_fwd<V: ApplyView>(
    view: &mut V,
    step: &StepKind,
    input: &STAmount,
    max_out: &STAmount,
    strand_src: &AccountID,
    quality_threshold: Option<Quality>,
) -> Result<(STAmount, STAmount), Ter> {
    let result = match step {
        StepKind::Direct { src, dst, .. } => execute_direct_fwd(view, src, dst, input),
        StepKind::XrpEndpoint { account, is_last } => {
            execute_xrp_endpoint_fwd(view, account, *is_last, input)
        }
        StepKind::Book { book_in, book_out } => execute_book_fwd(
            view,
            book_in,
            book_out,
            input,
            max_out,
            strand_src,
            quality_threshold,
        ),
    };

    if let Err(ref ter) = result {
        static STEP_ERR_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if STEP_ERR_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
            tracing::debug!(target: "ledger",                "[flow_step_err] step={:?} ter={:?} input_signum={} input_native={}",
                step,
                ter,
                input.signum(),
                input.native(),
            );
        }
    }

    result
}

/// Limits delivery to the trust line capacity (maxPaymentFlow), matching
/// Returns (consumed_in, delivered_out).
fn execute_direct_fwd<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    input: &STAmount,
) -> Result<(STAmount, STAmount), Ter> {
    if input.signum() <= 0 {
        return Ok((input.zeroed(), input.zeroed()));
    }

    // DirectStep only handles IOU. XRP is handled by XrpEndpointStep.
    if input.native() {
        let result = ripple_state_helpers::account_send(view, src, dst, input);
        if result != Ter::TES_SUCCESS {
            return Err(result);
        }
        return Ok((input.clone(), input.clone()));
    }

    let currency = input.issue().currency;

    // This is what reference does in the reverse pass to determine capacity.
    // We do it inline here since we don't have a separate reverse pass.
    let (max_flow, debt_dir) =
        crate::domain::ripple_calc::direct_step::max_payment_flow(view, src, dst, currency);

    if max_flow.is_zero() || max_flow.signum() <= 0 {
        static DIRECT_DRY_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if DIRECT_DRY_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
            tracing::debug!(target: "ledger",                "[direct_step] DRY: max_flow_signum={} input_signum={} currency_native={}",
                max_flow.signum(),
                input.signum(),
                input.native()
            );
        }
        return Ok((input.zeroed(), input.zeroed()));
    }

    // Limit delivery to available capacity
    let input_iou = input.iou();
    let deliver_iou = if input_iou > max_flow {
        static DIRECT_LIMIT_LOG: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);
        if DIRECT_LIMIT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
            tracing::debug!(target: "ledger",                "[direct_step] LIMITING: input_m={} input_e={} max_flow_m={} max_flow_e={}",
                input.mantissa(),
                input.exponent(),
                max_flow.mantissa(),
                max_flow.exponent()
            );
        }
        max_flow
    } else {
        input_iou
    };
    // When src redeems (positive balance), dst is the issuer (src is paying back).
    // When src issues (zero/negative balance), src is the issuer (src creates IOUs).
    let step_issue = Issue {
        currency,
        account: if debt_dir == crate::domain::ripple_calc::direct_step::DebtDirection::Redeems {
            *dst
        } else {
            *src
        },
    };
    let deliver = protocol::STAmount::from_iou_amount(
        protocol::get_field_by_symbol("sfAmount"),
        deliver_iou,
        step_issue,
    );

    if deliver.signum() <= 0 {
        return Ok((input.zeroed(), input.zeroed()));
    }

    let result = ripple_state_helpers::account_send(view, src, dst, &deliver);
    if result != Ter::TES_SUCCESS {
        return Err(result);
    }
    Ok((deliver.clone(), deliver))
}

fn execute_xrp_endpoint_fwd<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    is_last: bool,
    input: &STAmount,
) -> Result<(STAmount, STAmount), Ter> {
    let drops = input.xrp().drops();
    if drops <= 0 {
        return Ok((
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
        ));
    }

    // For source endpoint: limit by available XRP (balance - reserve)
    let actual_drops = if !is_last {
        let available = xrp_liquid(view, account);
        drops.min(available)
    } else {
        drops
    };

    if actual_drops <= 0 {
        return Ok((
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
        ));
    }

    // reference: accountSend(sb, sender, receiver, toSTAmount(result))
    let sender = if is_last { xrp_account() } else { *account };
    let receiver = if is_last { *account } else { xrp_account() };
    let ter = ripple_state_helpers::transfer_xrp(
        view,
        &sender,
        &receiver,
        XRPAmount::from_drops(actual_drops),
    );
    if ter != Ter::TES_SUCCESS {
        return Err(ter);
    }

    let amt = STAmount::from_xrp_amount(XRPAmount::from_drops(actual_drops));
    Ok((amt.clone(), amt))
}

fn execute_book_fwd<V: ApplyView>(
    view: &mut V,
    book_in: &Issue,
    book_out: &Issue,
    input: &STAmount,
    max_out: &STAmount,
    strand_src: &AccountID,
    quality_threshold: Option<Quality>,
) -> Result<(STAmount, STAmount), Ter> {
    let book = crate::domain::ripple_calc::book_step::Book {
        r#in: protocol::Asset::Issue(*book_in),
        out: protocol::Asset::Issue(*book_out),
        domain: None,
    };
    // Use the actual max_out limit from the strand (TakerPays for OfferCreate,
    // dst_amount for payments). reference BookStep uses the strand's out-limit.
    // Previously used synthetic unlimited which caused over-delivery.
    let result = crate::domain::ripple_calc::book_step::execute_book_step(
        view,
        &book,
        input,
        max_out,
        false,
        Some(strand_src),
        quality_threshold,
    );
    if result.amount_out.signum() <= 0 {
        return Err(Ter::TEC_PATH_DRY);
    }
    Ok((result.amount_in, result.amount_out))
}

fn xrp_liquid<V: ApplyView>(view: &mut V, account: &AccountID) -> i64 {
    let acct_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
    let Some(sle) = view.peek(acct_keylet).ok().flatten() else {
        return 0;
    };
    let balance = sle.get_field_amount(sf("sfBalance")).xrp().drops();
    let owner_count = sle.get_field_u32(sf("sfOwnerCount"));
    let reserve = view.fees().account_reserve(owner_count as usize) as i64;
    (balance - reserve).max(0)
}
