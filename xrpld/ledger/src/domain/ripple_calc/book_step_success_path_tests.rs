//! Success-path regressions for generic OfferCreate book consumption.
//!
//! Source parity: rippled `src/libxrpl/tx/paths/BookStep.cpp::limitStepIn`
//! operates on generic `TIn`/`TOut` amounts. It must not assume that a
//! limited book input is native XRP.

use super::*;

fn iou_amount(value: i64, issue: protocol::Issue) -> STAmount {
    STAmount::from_iou_amount(
        sf("sfAmount"),
        protocol::IOUAmount::from_parts(value, 0).expect("positive canonical IOU amount"),
        issue,
    )
}

#[test]
fn brrl_to_rlusd_limited_input_is_a_generic_iou_book_fill() {
    // Values are the canonical 2FA0… OfferCreate's BRRL/RLUSD shape. A
    // smaller remaining BRRL input takes the BookStep `limitStepIn` branch.
    let brrl = protocol::Issue::new(
        protocol::currency_from_string("BRRL"),
        AccountID::from_array([0xB1; 20]),
    );
    let rlusd = protocol::Issue::new(
        protocol::currency_from_string("RLUSD"),
        AccountID::from_array([0xC2; 20]),
    );
    let remaining_brrl = iou_amount(1_000, brrl);
    let available_rlusd = iou_amount(50_000, rlusd);

    let offer_in = iou_amount(255_960, brrl);
    let offer_quality =
        Quality::from_amounts(&Amounts::new(offer_in.clone(), available_rlusd.clone()));
    let consumed = compute_offer_consumption(
        BookStepPass::Forward,
        &remaining_brrl,
        &available_rlusd,
        &offer_in,
        &available_rlusd,
        &available_rlusd,
        offer_quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );

    assert_eq!(consumed.step_in, remaining_brrl);
    assert_eq!(consumed.step_in.asset(), Asset::Issue(brrl));
    assert_eq!(consumed.offer_in.asset(), Asset::Issue(brrl));
    assert_eq!(consumed.step_out.asset(), Asset::Issue(rlusd));
    assert_eq!(consumed.offer_out.asset(), Asset::Issue(rlusd));
    assert!(consumed.step_out.signum() > 0);
}

#[test]
fn forward_consumption_defers_output_cap_for_reverse_reconciliation() {
    let offer_in = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let offer_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let cache_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(50));
    let quality = Quality::from_amounts(&Amounts::new(offer_in.clone(), offer_out.clone()));

    let forward = compute_offer_consumption(
        BookStepPass::Forward,
        &offer_in,
        &cache_out,
        &offer_in,
        &offer_out,
        &offer_out,
        quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );
    let reverse = compute_offer_consumption(
        BookStepPass::Reverse,
        &offer_in,
        &cache_out,
        &offer_in,
        &offer_out,
        &offer_out,
        quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );

    assert_eq!(forward.step_in, offer_in);
    assert_eq!(forward.step_out, offer_out);
    assert_eq!(reverse.step_in.xrp().drops(), 50);
    assert_eq!(reverse.step_out, cache_out);
}

#[test]
fn output_only_reconciliation_preserves_forward_consumption_on_input_mismatch() {
    let remaining_in = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let offer_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let remaining_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(50));
    let quality = Quality::from_amounts(&Amounts::new(remaining_in.clone(), offer_out.clone()));

    let forward = compute_offer_consumption(
        BookStepPass::Forward,
        &remaining_in,
        &remaining_out,
        &remaining_in,
        &offer_out,
        &offer_out,
        quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );
    let output_only = compute_offer_consumption(
        BookStepPass::OutputOnly,
        &remaining_in,
        &remaining_out,
        &remaining_in,
        &offer_out,
        &offer_out,
        quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );

    // limitStepOut requires only 50 input. It must not be input-capped back
    // to 100, which would falsely satisfy the reconciliation equality check.
    assert_eq!(output_only.step_in.xrp().drops(), 50);
    assert_ne!(output_only.step_in, remaining_in);
    assert_eq!(forward.step_in, remaining_in);
    assert_eq!(forward.step_out, offer_out);
}

#[test]
fn accepted_reverse_cache_reconciliation_uses_exact_remaining_step_amounts() {
    let offer_in = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let offer_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
    let remaining_in = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(49));
    let remaining_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(49));
    let quality = Quality::from_amounts(&Amounts::new(offer_in.clone(), offer_out.clone()));

    // limitStepOut's quality arithmetic may return a normalized/rounded
    // amount. rippled still sets the accepted reconciliation's step accounting
    // to the exact outstanding cache amounts after it confirms the input.
    let mut consumption = compute_offer_consumption(
        BookStepPass::OutputOnly,
        &remaining_in,
        &remaining_out,
        &offer_in,
        &offer_out,
        &offer_out,
        quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );
    assert_eq!(consumption.step_in, remaining_in);

    // Model the one-ulp rounded output which the accepted cache boundary must
    // replace (the case that otherwise leaves a false FOK residual).
    consumption.step_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(48));
    assert_ne!(consumption.step_out, remaining_out);
    consumption.reconcile_step_to_cache(&remaining_in, &remaining_out);
    assert_eq!(consumption.step_in, remaining_in);
    assert_eq!(consumption.step_out, remaining_out);
}
