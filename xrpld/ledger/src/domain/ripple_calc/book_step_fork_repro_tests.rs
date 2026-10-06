//! Deterministic reproduction of the testnet OfferCreate IOU-crossing fork
//! (tx 1567D837…, validated network ledger 21305409). The observer built a
//! ledger with the SAME affected entries and tesSUCCESS but a DIFFERENT value,
//! localized to this crossing arithmetic. Ground-truth outputs are the
//! network's metadata deltas.
//!
//! Taker offer: TakerGets 4000000 drops (XRP), TakerPays 2.04693805346036 WAR.
//! Consumed offer (owner rapS8E…): TakerGets(WAR) 3.076560200591725 ->
//! 1.029622147131365 ; TakerPays(XRP drops) 6000000 -> 2008000.
//! => offer gives 2.04693805346036 WAR, receives 3992000 drops.

use super::*;

fn war_issue() -> protocol::Issue {
    protocol::Issue::new(
        protocol::currency_from_string("WAR"),
        AccountID::from_array([0x1A; 20]),
    )
}

/// Build an IOU STAmount from a decimal string with up to 16 significant
/// digits, e.g. "3.076560200591725".
fn war(value_str: &str) -> STAmount {
    let issue = war_issue();
    // Parse "<int>.<frac>" into mantissa*10^-frac_len.
    let neg = value_str.starts_with('-');
    let s = value_str.trim_start_matches('-');
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    let digits: String = format!("{int_part}{frac_part}");
    let mut mantissa: i64 = digits.parse().expect("mantissa digits");
    if neg {
        mantissa = -mantissa;
    }
    let exponent = -(frac_part.len() as i32);
    STAmount::from_iou_amount(
        sf("sfAmount"),
        protocol::IOUAmount::from_parts(mantissa, exponent).expect("canonical IOU amount"),
        issue,
    )
}

fn drops(n: i64) -> STAmount {
    STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(n))
}

#[test]
fn fork_21305409_trustline_balance_addition_canonicalizes_like_network() {
    // Network RippleState balance transitions for this crossing (16 sig digits):
    //   taker line: -414.6200709596663 + (-2.04693805346036) = -416.6670090131267
    //   owner line:  184.2149045765512 + (-2.04693805346036) =  182.1679665230908
    // A 1-ULP IOU add/canonicalize divergence here forks the ledger even though
    // the crossing step amounts match the network.
    let delta = war("-2.04693805346036");

    let taker_before = war("-414.6200709596663");
    let taker_after = taker_before + delta.clone();
    assert_eq!(
        taker_after,
        war("-416.6670090131267"),
        "taker trustline balance must canonicalize exactly like the network"
    );

    let owner_before = war("184.2149045765512");
    let owner_after = owner_before + delta.clone();
    assert_eq!(
        owner_after,
        war("182.1679665230908"),
        "owner trustline balance must canonicalize exactly like the network"
    );
}

#[test]
fn fork_21305409_offercreate_iou_crossing_matches_network() {
    // Consumed offer as it stood BEFORE this crossing.
    let offer_pays_xrp = drops(6_000_000); // owner receives XRP (offer "in")
    let offer_gets_war = war("3.076560200591725"); // owner gives WAR (offer "out")
    let offer_quality =
        Quality::from_amounts(&Amounts::new(offer_pays_xrp.clone(), offer_gets_war.clone()));

    // Taker is paying XRP to receive WAR. Its offer wants 2.04693805346036 WAR
    // and offers 4000000 drops. Step input = XRP, step output = WAR.
    let remaining_in_xrp = drops(4_000_000);
    let remaining_out_war = war("2.04693805346036");

    let consumed = compute_offer_consumption(
        BookStepPass::Reverse,
        &remaining_in_xrp,
        &remaining_out_war,
        &offer_pays_xrp,  // taker_pays == offer "in" (XRP)
        &offer_gets_war,  // taker_gets == offer "out" (WAR)
        &offer_gets_war,  // owner_funds: assume fully funded for this repro
        offer_quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true, // fixReducedOffersV2 enabled on the network
    );

    // Ground truth from network metadata:
    //  - WAR delivered to taker = 2.04693805346036
    //  - XRP paid by taker = 3992000 drops
    //  - offer remainder: TakerGets(WAR) 1.029622147131365, TakerPays 2008000
    let expected_war_out = war("2.04693805346036");
    let expected_xrp_in = drops(3_992_000);

    println!(
        "REPRO step_in={:?} step_out={:?} offer_in={:?} offer_out={:?} owner_gives={:?}",
        consumed.step_in, consumed.step_out, consumed.offer_in, consumed.offer_out,
        consumed.owner_gives
    );
    assert_eq!(
        consumed.step_out, expected_war_out,
        "WAR delivered must match network (2.04693805346036)"
    );
    assert_eq!(
        consumed.step_in, expected_xrp_in,
        "XRP paid must match network (3992000 drops)"
    );
}


/// Build a 524C IOU STAmount from a decimal string.
fn c524(value_str: &str) -> STAmount {
    let issue = protocol::Issue::new(
        protocol::currency_from_string("524C"),
        AccountID::from_array([0x52; 20]),
    );
    let neg = value_str.starts_with('-');
    let s = value_str.trim_start_matches('-');
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    let digits: String = format!("{int_part}{frac_part}");
    let mut mantissa: i64 = digits.parse().expect("mantissa digits");
    if neg {
        mantissa = -mantissa;
    }
    let exponent = -(frac_part.len() as i32);
    STAmount::from_iou_amount(
        sf("sfAmount"),
        protocol::IOUAmount::from_parts(mantissa, exponent).expect("canonical IOU amount"),
        issue,
    )
}

/// Reproduction of fork 21332266 (binary widetrace-1ae45356): a tfSell
/// OfferCreate crossing a resting self-issued-524C offer produced a +-1 drop
/// XRP balance divergence vs the network. Captured CLOB_OFFER_CONSUMPTION:
///   resting offer 2E92317F: TakerGets=5999993 drops XRP (offer "out"),
///   TakerPays=3.039980301943390 524C (offer "in"); owner_funds large;
///   tfSell crossing wants remaining_out=4000000 drops XRP; our node consumed
///   cons_out=4000000 XRP for cons_in=2.026655899060810 524C.
/// This test runs the exact consumption and prints every intermediate so the
/// +-1 drop can be bisected against rippled's BookStep.
#[test]
fn fork_21332266_tfsell_xrp_out_crossing_amounts() {
    let offer_in_524 = c524("3.039980301943390"); // owner receives 524C (offer "in")
    let offer_out_xrp = drops(5_999_993); // owner gives XRP (offer "out")
    let offer_quality =
        Quality::from_amounts(&Amounts::new(offer_in_524.clone(), offer_out_xrp.clone()));

    // tfSell crossing: step in = 524C, step out = XRP, want 4,000,000 drops out.
    let remaining_in_524 = c524("1000000000000000000"); // effectively unbounded (tfSell)
    let remaining_out_xrp = drops(4_000_000);

    let consumed = compute_offer_consumption(
        BookStepPass::Reverse,
        &remaining_in_524,
        &remaining_out_xrp,
        &offer_in_524,  // taker_pays == offer "in" (524C)
        &offer_out_xrp, // taker_gets == offer "out" (XRP)
        &offer_out_xrp, // owner_funds: fully funded for this repro (XRP side)
        offer_quality,
        QUALITY_ONE,
        QUALITY_ONE,
        true,
    );

    // Also run the raw ceil_out_strict that the remaining_out clip uses, to
    // isolate the IOU rounding of the input side for 4,000,000 XRP out.
    let clip = offer_quality.ceil_out_strict(
        &Amounts::new(offer_in_524.clone(), offer_out_xrp.clone()),
        &remaining_out_xrp,
        true,
    );
    println!(
        "REPRO 21332266 ceil_out_strict(roundUp=true): in(524C)={:?} out(XRP)={:?}",
        clip.r#in, clip.out
    );
    println!(
        "REPRO 21332266 consumed: step_in(524C)={:?} step_out(XRP)={:?} offer_in={:?} offer_out={:?} owner_gives={:?}",
        consumed.step_in, consumed.step_out, consumed.offer_in, consumed.offer_out, consumed.owner_gives
    );
    // Our node produced cons_out=4000000 XRP and cons_in=2.026655899060810 524C.
    assert_eq!(consumed.step_out, drops(4_000_000), "XRP out must be 4000000 drops");
    // Record our current 524C input; the network's value is the comparison target.
    assert_eq!(
        consumed.step_in,
        c524("2.026655899060810"),
        "our captured 524C input (compare to network ground truth)"
    );
}


/// Reproduction + regression for the NFTokenAcceptOffer royalty-split +-1-drop
/// fork (fork 21332266, tx DD164285). Network split a 5,711,412-drop sale with a
/// 5% (5000-bps) transfer fee into royalty=285571 (issuer) + seller=5425841.
/// Our previous floor `mul_ratio` produced royalty=285570 (5711412*0.05 =
/// 285570.6 floored), forking by 1 drop. rippled uses
/// `multiply(amount, transferFeeAsRate(fee))` which canonicalize-rounds to
/// 285571. This asserts the parity helper matches the network.
#[test]
fn nft_royalty_cut_canonicalize_rounds_like_network() {
    let gross = drops(5_711_412);
    let rate = protocol::rate::nft::transfer_fee_as_rate(5000);
    let cut = protocol::rate::multiply_rate(&gross, rate);
    assert_eq!(
        cut, drops(285_571),
        "NFT royalty cut must canonicalize-round to 285571 (network), not floor to 285570"
    );
    // Seller proceeds = gross - cut must match the network's 5425841.
    let seller = gross - cut;
    assert_eq!(seller, drops(5_425_841), "seller proceeds must match network");
}
