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
