//! RippleCalc engine — port of reference `path::RippleCalc::rippleCalculate`.

mod builder;

mod selection;
mod step;
mod strand;

use crate::read_view::ViewError;
use crate::views::apply_view::ApplyView;
use basics::base_uint::Uint160;
use protocol::{
    AccountID, STAmount, STLedgerEntry, STPathSet, Ter, get_field_by_symbol, is_tes_success,
};
use std::sync::Arc;

use self::selection::rank_explicit_strands;
use self::strand::{build_explicit_strand, execute_direct_strand};

#[derive(Debug, Clone)]
pub struct RippleCalcInput {
    pub partial_payment_allowed: bool,
    pub default_paths_allowed: bool,
    pub limit_quality: bool,
    pub is_ledger_open: bool,
}

#[derive(Debug, Clone)]
pub struct RippleCalcOutput {
    pub result: Ter,
    pub actual_amount_in: STAmount,
    pub actual_amount_out: STAmount,
}

const QUALITY_ONE: u32 = 1_000_000_000;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_to_uint160(account: &AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

/// Handle XRP→XRP payments that go through rippleCalculate.
/// In reference, the flow engine builds a strand with XRP endpoint steps and
/// executes a direct transfer. We replicate that behavior here.
fn handle_xrp_to_xrp_flow<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    input: &RippleCalcInput,
) -> Result<RippleCalcOutput, ViewError> {
    // The amount to deliver is min(dst_amount, max_source_amount).
    let deliver = if dst_amount.xrp().drops() <= max_source_amount.xrp().drops() {
        dst_amount.clone()
    } else if input.partial_payment_allowed {
        max_source_amount.clone()
    } else {
        // Can't deliver full amount and partial not allowed
        return Ok(RippleCalcOutput {
            result: Ter::TEC_PATH_DRY,
            actual_amount_in: max_source_amount.zeroed(),
            actual_amount_out: dst_amount.zeroed(),
        });
    };

    let deliver_drops = deliver.xrp().drops();
    if deliver_drops <= 0 {
        return Ok(RippleCalcOutput {
            result: Ter::TEC_PATH_DRY,
            actual_amount_in: max_source_amount.zeroed(),
            actual_amount_out: dst_amount.zeroed(),
        });
    }

    // Check source has sufficient balance (after fee was already deducted)
    let src_keylet = protocol::account_keylet(account_to_uint160(src_account));
    let Some(src_sle) = view.peek(src_keylet)? else {
        return Ok(RippleCalcOutput {
            result: Ter::TER_NO_ACCOUNT,
            actual_amount_in: max_source_amount.zeroed(),
            actual_amount_out: dst_amount.zeroed(),
        });
    };
    let src_balance = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
    if src_balance < deliver_drops {
        if input.partial_payment_allowed && src_balance > 0 {
            // Partial: deliver what we can
            let actual_deliver = src_balance;
            // Transfer XRP
            let mut src_obj = src_sle.clone_as_object();
            src_obj.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(
                    src_balance - actual_deliver,
                )),
            );
            let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
                src_obj,
                *src_sle.key(),
            )));

            let dst_keylet = protocol::account_keylet(account_to_uint160(dst_account));
            if let Some(dst_sle) = view.peek(dst_keylet)? {
                let dst_balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let mut dst_obj = dst_sle.clone_as_object();
                dst_obj.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(
                        dst_balance + actual_deliver,
                    )),
                );
                let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
                    dst_obj,
                    *dst_sle.key(),
                )));
            }

            return Ok(RippleCalcOutput {
                result: Ter::TES_SUCCESS,
                actual_amount_in: STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(
                    actual_deliver,
                )),
                actual_amount_out: STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(
                    actual_deliver,
                )),
            });
        }
        return Ok(RippleCalcOutput {
            result: Ter::TEC_PATH_DRY,
            actual_amount_in: max_source_amount.zeroed(),
            actual_amount_out: dst_amount.zeroed(),
        });
    }

    // Transfer XRP: debit source, credit destination
    let mut src_obj = src_sle.clone_as_object();
    src_obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(src_balance - deliver_drops)),
    );
    let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
        src_obj,
        *src_sle.key(),
    )));

    let dst_keylet = protocol::account_keylet(account_to_uint160(dst_account));
    if let Some(dst_sle) = view.peek(dst_keylet)? {
        let dst_balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut dst_obj = dst_sle.clone_as_object();
        dst_obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(dst_balance + deliver_drops)),
        );
        let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
            dst_obj,
            *dst_sle.key(),
        )));
    }

    Ok(RippleCalcOutput {
        result: Ter::TES_SUCCESS,
        actual_amount_in: deliver.clone(),
        actual_amount_out: deliver,
    })
}

pub fn ripple_calculate<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    paths: &STPathSet,
    input: &RippleCalcInput,
) -> Result<RippleCalcOutput, ViewError> {
    // steps. For parity, handle direct XRP→XRP transfers here before falling
    // through to the IOU flow engine.
    if dst_amount.native() && max_source_amount.native() {
        return handle_xrp_to_xrp_flow(
            view,
            max_source_amount,
            dst_amount,
            dst_account,
            src_account,
            input,
        );
    }

    // Debug: log what reaches the IOU flow engine
    static FLOW_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let flow_c = FLOW_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if flow_c < 30 {
        tracing::debug!(target: "ledger",            "[ripple_calc] src_native={} dst_native={} paths={} default_allowed={}",
            max_source_amount.native(),
            dst_amount.native(),
            paths.size(),
            input.default_paths_allowed,
        );
    }

    // Only apply the sandbox to the parent view on tesSUCCESS.
    // On failure, the sandbox is discarded (no state changes).
    let mut flow_sb = crate::FlowSandbox::new(view);
    let result = ripple_calculate_inner(
        &mut flow_sb,
        max_source_amount,
        dst_amount,
        dst_account,
        src_account,
        paths,
        input,
    )?;

    if is_tes_success(result.result) {
        flow_sb.apply()?;
    }
    // On failure, flow_sb is dropped — all changes discarded

    Ok(result)
}

/// Inner flow engine logic that operates on the FlowSandbox.
/// All state changes are captured in the sandbox and only committed by the caller on success.
fn ripple_calculate_inner<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    paths: &STPathSet,
    input: &RippleCalcInput,
) -> Result<RippleCalcOutput, ViewError> {
    let mut total_in = max_source_amount.zeroed();
    let mut total_out = dst_amount.zeroed();

    // This replaces the simplified try_default_path approach.
    let deliver_asset = dst_amount.asset();
    let send_max_asset = if max_source_amount.asset() != dst_amount.asset() {
        Some(max_source_amount.asset())
    } else {
        None
    };

    let (strand_ter, strands) = crate::domain::flow_engine::strand_builder::to_strands(
        src_account,
        dst_account,
        &deliver_asset,
        send_max_asset.as_ref(),
        paths,
        input.default_paths_allowed,
        false, // ownerPaysTransferFee = false for payments
        false, // offerCrossing = false for payments
    );

    if is_tes_success(strand_ter) && !strands.is_empty() {
        // Flow engine runs in its own sandbox. If it fails, changes are discarded
        // and the fallback runs on a clean view.
        let mut engine_sb = crate::FlowSandbox::new(view);
        let flow_result = crate::domain::flow_engine::strand_flow::execute_strands(
            &mut engine_sb,
            &strands,
            dst_amount,
            input.partial_payment_allowed,
            Some(max_source_amount),
            src_account,
            None, // payments have no quality threshold
        );

        static FLOW_RESULT_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if FLOW_RESULT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 30 {
            tracing::debug!(target: "ledger",                "[flow_engine] strands={} ter={:?} out_signum={} in_signum={} src_native={} dst_native={} default_allowed={}",
                strands.len(),
                flow_result.ter,
                flow_result.actual_out.signum(),
                flow_result.actual_in.signum(),
                max_source_amount.native(),
                dst_amount.native(),
                input.default_paths_allowed,
            );
        }

        if is_tes_success(flow_result.ter) && flow_result.actual_out.signum() > 0 {
            // Apply flow engine changes to the view
            let _ = engine_sb.apply();
            return Ok(RippleCalcOutput {
                result: flow_result.ter,
                actual_amount_in: flow_result.actual_in,
                actual_amount_out: flow_result.actual_out,
            });
        }
        // Flow engine found strands but delivered nothing.
        // When default paths are NOT allowed (tfNoRippleDirect), the fallback
        // would use the default path which is forbidden — return TEC_PATH_DRY.
        // When default paths ARE allowed, let the fallback try (it may find
        // liquidity through a different code path).
        if !input.default_paths_allowed {
            return Ok(RippleCalcOutput {
                result: Ter::TEC_PATH_DRY,
                actual_amount_in: max_source_amount.zeroed(),
                actual_amount_out: dst_amount.zeroed(),
            });
        }
        // Flow engine failed — engine_sb is dropped, view unchanged
    }

    // Fallback to existing simplified paths if flow engine didn't produce results.
    // WARNING: This fallback may produce different results from reference. It should
    // only fire for edge cases that to_strand doesn't handle yet.
    static FALLBACK_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if FALLBACK_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
        tracing::debug!(target: "ledger",            "[ripple_calc_FALLBACK] flow engine failed, using fallback. src_native={} dst_native={} paths={}",
            max_source_amount.native(),
            dst_amount.native(),
            paths.size(),
        );
    }

    // 1. Try default path
    if input.default_paths_allowed
        && let Ok(Some(res)) = try_default_path(
            view,
            max_source_amount,
            dst_amount,
            dst_account,
            src_account,
            input,
        )
        && res.result == Ter::TES_SUCCESS
    {
        total_in += res.actual_amount_in.clone();
        total_out += res.actual_amount_out.clone();
        if total_out >= *dst_amount && !input.partial_payment_allowed {
            return Ok(res);
        }
    }

    // 2. Try explicit paths using deterministic strand ranking.
    let mut explicit_strands = paths
        .iter()
        .enumerate()
        .filter_map(|(path_index, path)| {
            build_explicit_strand(
                path_index,
                src_account,
                dst_account,
                max_source_amount,
                dst_amount,
                path,
            )
        })
        .collect::<Vec<_>>();

    // BookStep strands. Our build_explicit_strand only handles same-currency IOU paths.
    // For cross-currency explicit paths, use the cross-currency path functions which
    // match reference toStrand's BookStep strand building.
    // Only use the default path if default_paths_allowed (i.e. tfNoRippleDirect is NOT set).
    if explicit_strands.is_empty()
        && paths.size() > 0
        && total_out.signum() <= 0
        && input.default_paths_allowed
    {
        if max_source_amount.native() && !dst_amount.native() {
            if let Ok(Some(res)) = try_xrp_to_iou_default_path(
                view,
                max_source_amount,
                dst_amount,
                dst_account,
                src_account,
                input,
            ) && res.result == Ter::TES_SUCCESS
            {
                total_in += res.actual_amount_in.clone();
                total_out += res.actual_amount_out.clone();
            }
        } else if !max_source_amount.native()
            && dst_amount.native()
            && let Ok(Some(res)) = try_iou_to_xrp_default_path(
                view,
                max_source_amount,
                dst_amount,
                dst_account,
                src_account,
                input,
            )
            && res.result == Ter::TES_SUCCESS
        {
            total_in += res.actual_amount_in.clone();
            total_out += res.actual_amount_out.clone();
        }
    }

    const MAX_TRIES: usize = 1000;
    let mut cur_try: usize = 0;

    while !explicit_strands.is_empty() {
        if total_out >= *dst_amount && !input.partial_payment_allowed {
            break;
        }
        cur_try += 1;
        if cur_try >= MAX_TRIES {
            break;
        }

        let remaining_out = dst_amount.clone() - total_out.clone();
        let remaining_in = max_source_amount.clone() - total_in.clone();
        let ranked = rank_explicit_strands(
            view,
            &explicit_strands,
            &remaining_in,
            &remaining_out,
            input,
        )?;
        if ranked.is_empty() {
            break;
        }

        let mut applied = false;
        for ranked_strand in ranked {
            let Some(position) = explicit_strands
                .iter()
                .position(|strand| strand.path_index == ranked_strand.path_index)
            else {
                continue;
            };

            let path = explicit_strands[position].path.clone();

            // In a real implementation, SnapshotReadView would be used for sandboxing
            // For now, use the view directly with sub-view if supported
            if let Some(res) = try_explicit_path(
                view,
                &remaining_in,
                &remaining_out,
                dst_account,
                src_account,
                &path,
                input,
            )? && is_tes_success(res.result)
            {
                total_in += res.actual_amount_in.clone();
                total_out += res.actual_amount_out.clone();
                applied = true;
            }

            explicit_strands.remove(position);
            if applied {
                break;
            }
        }

        if !applied {
            break;
        }
    }

    if total_out.signum() > 0 {
        Ok(RippleCalcOutput {
            result: Ter::TES_SUCCESS,
            actual_amount_in: total_in,
            actual_amount_out: total_out,
        })
    } else {
        // tecPATH_DRY is for when the path is structurally dry (no liquidity at all).
        // tecPATH_PARTIAL is for when the path has some structure but can't deliver the full amount.
        // The payment transactor maps terRetry → tecPATH_DRY, so we use PATH_PARTIAL here
        // to match reference behavior when the flow engine found strands but delivered nothing.
        let result = if input.partial_payment_allowed {
            Ter::TEC_PATH_DRY
        } else {
            Ter::TEC_PATH_PARTIAL
        };
        Ok(RippleCalcOutput {
            result,
            actual_amount_in: total_in,
            actual_amount_out: total_out,
        })
    }
}

fn try_default_path<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    input: &RippleCalcInput,
) -> Result<Option<RippleCalcOutput>, ViewError> {
    // Exception: issuer is never frozen for their own tokens
    if !dst_amount.native() {
        let issue = dst_amount.issue();
        if *src_account != issue.account
            && *dst_account != issue.account
            && (crate::domain::ripple_state_helpers::is_frozen(view, src_account, &issue)
                || crate::domain::ripple_state_helpers::is_frozen(view, dst_account, &issue))
        {
            return Ok(Some(RippleCalcOutput {
                result: Ter::TEC_PATH_DRY,
                actual_amount_in: max_source_amount.zeroed(),
                actual_amount_out: dst_amount.zeroed(),
            }));
        }
    }

    // XRP→IOU: cross DEX order book
    if max_source_amount.native() && !dst_amount.native() {
        return try_xrp_to_iou_default_path(
            view,
            max_source_amount,
            dst_amount,
            dst_account,
            src_account,
            input,
        );
    }
    // IOU→XRP: cross DEX order book
    if !max_source_amount.native() && dst_amount.native() {
        return try_iou_to_xrp_default_path(
            view,
            max_source_amount,
            dst_amount,
            dst_account,
            src_account,
            input,
        );
    }
    if dst_amount.native() || max_source_amount.native() {
        return Ok(None);
    }

    let issue = dst_amount.issue();
    let issuer = issue.account;
    let currency = issue.currency;

    // Case 1: sender IS issuer → direct to receiver
    // Case 2: receiver IS issuer → direct from sender
    // Case 3: neither is issuer → route through issuer (two hops)
    // Case 4: direct trust line exists → direct transfer

    let has_direct_line = view
        .peek(protocol::line(*src_account, *dst_account, currency))?
        .is_some();
    let src_is_issuer = *src_account == issuer;
    let dst_is_issuer = *dst_account == issuer;

    if !has_direct_line && !src_is_issuer && !dst_is_issuer {
        // so we don't need to pre-check for their existence.

        // Route through issuer
        let rate = {
            let issuer_keylet = protocol::account_keylet(account_to_uint160(&issuer));
            if let Some(issuer_sle) = view.read(issuer_keylet)? {
                let r = issuer_sle.get_field_u32(sf("sfTransferRate"));
                if r == 0 { QUALITY_ONE } else { r }
            } else {
                QUALITY_ONE
            }
        };

        let amount_to_deliver = dst_amount.clone();
        let amount_needed = if rate == QUALITY_ONE {
            amount_to_deliver.clone()
        } else {
            let rate_amount = STAmount::new_with_asset(
                sf("sfAmount"),
                dst_amount.asset(),
                rate as u64,
                -9,
                false,
            );
            amount_to_deliver.multiply(&rate_amount, dst_amount.asset())
        };

        if amount_needed > *max_source_amount {
            if input.partial_payment_allowed {
                let actual_in = max_source_amount.clone();
                let rate_amount = STAmount::new_with_asset(
                    sf("sfAmount"),
                    dst_amount.asset(),
                    rate as u64,
                    -9,
                    false,
                );
                let actual_out = actual_in.divide(&rate_amount, dst_amount.asset());
                // Two-hop: sender→issuer→receiver
                apply_direct_iou_transfer(
                    view,
                    src_account,
                    dst_account,
                    &actual_out,
                    &issuer,
                    currency,
                )?;
                return Ok(Some(RippleCalcOutput {
                    result: Ter::TES_SUCCESS,
                    actual_amount_in: actual_in,
                    actual_amount_out: actual_out,
                }));
            }
            return Ok(None);
        }

        // Two-hop transfer through issuer
        apply_direct_iou_transfer(
            view,
            src_account,
            dst_account,
            &amount_to_deliver,
            &issuer,
            currency,
        )?;
        return Ok(Some(RippleCalcOutput {
            result: Ter::TES_SUCCESS,
            actual_amount_in: amount_needed,
            actual_amount_out: amount_to_deliver,
        }));
    }

    // Direct path (trust line exists, or one party is issuer)
    let rate = if src_is_issuer || dst_is_issuer {
        QUALITY_ONE
    } else {
        let issuer_keylet = protocol::account_keylet(account_to_uint160(&issuer));
        if let Some(issuer_sle) = view.read(issuer_keylet)? {
            let r = issuer_sle.get_field_u32(sf("sfTransferRate"));
            if r == 0 { QUALITY_ONE } else { r }
        } else {
            QUALITY_ONE
        }
    };

    let amount_to_deliver = dst_amount.clone();
    let amount_needed = if rate == QUALITY_ONE {
        amount_to_deliver.clone()
    } else {
        let rate_amount =
            STAmount::new_with_asset(sf("sfAmount"), dst_amount.asset(), rate as u64, -9, false);
        amount_to_deliver.multiply(&rate_amount, dst_amount.asset())
    };

    if amount_needed > *max_source_amount {
        if input.partial_payment_allowed {
            let actual_in = max_source_amount.clone();
            let rate_amount = STAmount::new_with_asset(
                sf("sfAmount"),
                dst_amount.asset(),
                rate as u64,
                -9,
                false,
            );
            let actual_out = actual_in.divide(&rate_amount, dst_amount.asset());
            apply_direct_iou_transfer(
                view,
                src_account,
                dst_account,
                &actual_out,
                &issuer,
                currency,
            )?;
            return Ok(Some(RippleCalcOutput {
                result: Ter::TES_SUCCESS,
                actual_amount_in: actual_in,
                actual_amount_out: actual_out,
            }));
        }
        return Ok(None);
    }

    apply_direct_iou_transfer(
        view,
        src_account,
        dst_account,
        &amount_to_deliver,
        &issuer,
        currency,
    )?;
    Ok(Some(RippleCalcOutput {
        result: Ter::TES_SUCCESS,
        actual_amount_in: amount_needed,
        actual_amount_out: amount_to_deliver,
    }))
}

/// XRP→IOU default path: source sends XRP, crosses XRP/IOU order book, delivers IOU.
fn try_xrp_to_iou_default_path<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    _input: &RippleCalcInput,
) -> Result<Option<RippleCalcOutput>, ViewError> {
    let dst_issue = dst_amount.issue();
    // Book: XRP → destination IOU
    let book = book_step::Book {
        r#in: protocol::xrp_issue(),
        out: dst_issue,
    };
    let result = book_step::execute_book_step(
        view,
        &book,
        max_source_amount,
        dst_amount,
        false,
        Some(src_account),
        None,
    );

    static XRP_IOU_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if XRP_IOU_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
        tracing::debug!(target: "ledger",            "[xrp_to_iou] book_result: ter={:?} in_signum={} out_signum={} offers_consumed={}",
            result.ter,
            result.amount_in.signum(),
            result.amount_out.signum(),
            result.offers_consumed
        );
    }

    if result.ter != Ter::TES_SUCCESS || result.amount_out.signum() <= 0 {
        return Ok(None);
    }

    // Debit XRP from source
    let src_keylet = protocol::account_keylet(account_to_uint160(src_account));
    if let Some(src_sle) = view.peek(src_keylet)? {
        let balance = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let debit = result.amount_in.xrp().drops();
        if debit > balance {
            return Ok(None);
        }
        let mut obj = src_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(balance - debit)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
    }

    // Credit IOU to destination (issuer→dst via trust line)
    let issuer = dst_issue.account;
    if *dst_account != issuer {
        let _ = crate::domain::ripple_state_helpers::direct_send_no_fee_iou_pub(
            view,
            &issuer,
            dst_account,
            &result.amount_out,
        );
    }

    Ok(Some(RippleCalcOutput {
        result: Ter::TES_SUCCESS,
        actual_amount_in: result.amount_in,
        actual_amount_out: result.amount_out,
    }))
}

/// IOU→XRP default path: source sends IOU, crosses IOU/XRP order book, delivers XRP.
fn try_iou_to_xrp_default_path<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    _input: &RippleCalcInput,
) -> Result<Option<RippleCalcOutput>, ViewError> {
    let src_issue = max_source_amount.issue();
    // Book: source IOU → XRP
    let book = book_step::Book {
        r#in: src_issue,
        out: protocol::xrp_issue(),
    };
    let result = book_step::execute_book_step(
        view,
        &book,
        max_source_amount,
        dst_amount,
        false,
        Some(src_account),
        None,
    );

    static IOU_XRP_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if IOU_XRP_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
        tracing::debug!(target: "ledger",            "[iou_to_xrp] book_result: ter={:?} in_signum={} out_signum={} offers_consumed={}",
            result.ter,
            result.amount_in.signum(),
            result.amount_out.signum(),
            result.offers_consumed
        );
    }

    if result.ter != Ter::TES_SUCCESS || result.amount_out.signum() <= 0 {
        return Ok(None);
    }

    // Debit IOU from source (src→issuer via trust line)
    let issuer = src_issue.account;
    if *src_account != issuer {
        let _ = crate::domain::ripple_state_helpers::direct_send_no_fee_iou_pub(
            view,
            src_account,
            &issuer,
            &result.amount_in,
        );
    }

    // Credit XRP to destination
    let dst_keylet = protocol::account_keylet(account_to_uint160(dst_account));
    if let Some(dst_sle) = view.peek(dst_keylet)? {
        let balance = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let credit = result.amount_out.xrp().drops();
        let mut obj = dst_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(balance + credit)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dst_sle.key())));
    }

    Ok(Some(RippleCalcOutput {
        result: Ter::TES_SUCCESS,
        actual_amount_in: result.amount_in,
        actual_amount_out: result.amount_out,
    }))
}

fn apply_direct_iou_transfer<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    receiver: &AccountID,
    amount: &STAmount,
    issuer: &AccountID,
    _currency: protocol::Currency,
) -> Result<(), ViewError> {
    if amount.signum() == 0 {
        return Ok(());
    }
    // If sender or receiver is issuer → single direct transfer (no fee)
    // If neither is issuer → two-leg transfer through issuer (fee applied by caller)
    if *sender == *issuer || *receiver == *issuer || issuer.data().iter().all(|&b| b == 0) {
        // Direct: sender → receiver (one of them is issuer)
        let _ = crate::domain::ripple_state_helpers::direct_send_no_fee_iou_pub(
            view, sender, receiver, amount,
        );
    } else {
        // 3rd party: issuer credits receiver, sender debits to issuer
        // The caller already computed the fee-adjusted amount for the sender side
        let _ = crate::domain::ripple_state_helpers::direct_send_no_fee_iou_pub(
            view, issuer, receiver, amount,
        );
        // Sender pays amount * rate to issuer (caller passes amount_to_deliver here,
        // but the actual debit from sender is amount_needed which includes fee)
        // Since try_default_path calls us with amount_to_deliver, we need to
        // compute the fee-adjusted amount for the sender→issuer leg
        let rate = crate::domain::ripple_state_helpers::transfer_rate(view, issuer);
        let sender_amount = if rate == 1_000_000_000 {
            amount.clone()
        } else {
            let iou = amount.iou();
            let adjusted = crate::domain::mul_ratio::mul_ratio(
                iou,
                rate,
                crate::domain::mul_ratio::QUALITY_ONE,
                true,
            );
            STAmount::from_iou_amount(sf("sfAmount"), adjusted, amount.issue())
        };
        let _ = crate::domain::ripple_state_helpers::direct_send_no_fee_iou_pub(
            view,
            sender,
            issuer,
            &sender_amount,
        );
    }
    Ok(())
}

#[allow(dead_code)]
fn credit_trust_line<V: ApplyView>(
    view: &mut V,
    issuer: &AccountID,
    holder: &AccountID,
    amount: &STAmount,
    currency: protocol::Currency,
) -> Result<(), ViewError> {
    let line_keylet = protocol::line(*issuer, *holder, currency);
    if let Some(sle) = view.peek(line_keylet)? {
        let b_high = *holder > *issuer;
        let current_balance = sle.get_field_amount(sf("sfBalance"));
        let new_balance = if b_high {
            current_balance.clone() - amount.clone()
        } else {
            current_balance.clone() + amount.clone()
        };
        let mut obj = sle.clone_as_object();
        obj.set_field_amount(sf("sfBalance"), new_balance);
        view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))?;
    }
    Ok(())
}

fn try_explicit_path<V: ApplyView>(
    view: &mut V,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    dst_account: &AccountID,
    src_account: &AccountID,
    path: &protocol::STPath,
    input: &RippleCalcInput,
) -> Result<Option<RippleCalcOutput>, ViewError> {
    let Some(strand) = build_explicit_strand(
        0,
        src_account,
        dst_account,
        max_source_amount,
        dst_amount,
        path,
    ) else {
        return Ok(None);
    };

    execute_direct_strand(
        view,
        &strand,
        max_source_amount,
        dst_amount,
        input.partial_payment_allowed,
    )
}
pub mod book_step;
pub mod direct_step;
pub mod xrp_endpoint_step;
