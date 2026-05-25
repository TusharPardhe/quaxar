//! Owner-account guard slice for the reference implementation.
//!
//! This helper preserves the the reference implementation guard ordering:
//!
//! - missing owner maps to `tefINTERNAL`,
//! - reserve shortfall maps before the unfunded check,
//! - and the destination check stays outside this helper.

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentChannelFundOwnerGuardFacts<Owner> {
    pub owner: Owner,
}

pub fn load_payment_channel_fund_owner_guard_facts<Owner, ReadOwner, HasReserve, HasFunds>(
    read_owner: ReadOwner,
    has_reserve: HasReserve,
    has_funds: HasFunds,
) -> Result<PaymentChannelFundOwnerGuardFacts<Owner>, Ter>
where
    ReadOwner: FnOnce() -> Option<Owner>,
    HasReserve: FnOnce(&Owner) -> bool,
    HasFunds: FnOnce(&Owner) -> bool,
{
    let owner = read_owner().ok_or(Ter::TEF_INTERNAL)?;

    if !has_reserve(&owner) {
        return Err(Ter::TEC_INSUFFICIENT_RESERVE);
    }

    if !has_funds(&owner) {
        return Err(Ter::TEC_UNFUNDED);
    }

    Ok(PaymentChannelFundOwnerGuardFacts { owner })
}
