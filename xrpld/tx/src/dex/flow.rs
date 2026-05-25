//! Multi-asset flow math ported from the reference implementation.

use protocol::STAmount;
use protocol::amounts::quality::mul_round;

/// Calculates the flow of assets through a path or book crossing.
pub fn calculate_flow(amount_in: &STAmount, quality: u64, round_up: bool) -> STAmount {
    // Ported from the reference source logic
    // Uses the STAmount multiplication/division with rounding modes
    // to ensure no value is lost or gained beyond protocol rules.
    mul_round(
        amount_in,
        &protocol::amount_from_quality(quality),
        amount_in.asset(),
        round_up,
    )
}
