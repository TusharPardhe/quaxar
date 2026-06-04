//! `AmmVote` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub const fn amm_vote_check_extra_features(
    amm_enabled: bool,
    mptokens_v2_enabled: bool,
    asset_holds_mpt: bool,
    asset2_holds_mpt: bool,
) -> bool {
    if !amm_enabled {
        return false;
    }

    if (asset_holds_mpt || asset2_holds_mpt) && !mptokens_v2_enabled {
        return false;
    }

    true
}

#[derive(Debug, Clone)]
pub struct AMMVotePreflightFacts {
    pub asset_pair_invalid: Option<NotTec>,
    pub trading_fee: u16,
}

pub fn run_amm_vote_preflight_facts(facts: AMMVotePreflightFacts) -> NotTec {
    if let Some(err) = facts.asset_pair_invalid {
        return err;
    }

    if facts.trading_fee > protocol::TRADING_FEE_THRESHOLD {
        return Ter::TEM_BAD_FEE;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy)]
pub struct AMMVotePreclaimFacts {
    pub amm_exists: bool,
    pub lp_token_balance_signum: i32,
    pub account_lp_holds_signum: Option<i32>,
}

pub fn run_amm_vote_preclaim_facts(facts: AMMVotePreclaimFacts) -> Ter {
    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if facts.lp_token_balance_signum == 0 {
        return Ter::TEC_AMM_EMPTY;
    }

    if facts.account_lp_holds_signum.unwrap_or(0) == 0 {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_vote_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    flags: u32,
) -> NotTec {
    let ret = run_transactor_preflight1(
        TransactorPreflight1Facts {
            inner_batch_flag_set: (ctx.flags.bits() & crate::ApplyFlags::BATCH.bits()) != 0,
            batch_enabled: ctx.rules.enabled(&feature_batch()),
            ..Default::default()
        },
        || {
            run_transactor_preflight0(
                TransactorPreflight0Facts {
                    tx_flags: flags,
                    ..Default::default()
                },
                0,
            )
        },
        || Ter::TES_SUCCESS,
    );
    if !is_tes_success(ret) {
        return ret;
    }
    Ter::TES_SUCCESS
}

pub fn run_amm_vote_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub struct AMMVoteApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
    pub trading_fee: u16,
    pub lp_token_balance: u128,
    pub account_lp_tokens: u128,
    pub vote_slots: Vec<AMMVoteSlot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AMMVoteSlot {
    pub account: protocol::AccountID,
    pub trading_fee: u16,
    pub lp_tokens: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMVoteApplyResult {
    pub vote_slots: Vec<AMMVoteResultSlot>,
    pub trading_fee: Option<u16>,
    pub discounted_fee: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AMMVoteResultSlot {
    pub account: protocol::AccountID,
    pub trading_fee: Option<u16>,
    pub vote_weight: u32,
}

fn vote_weight(lp_tokens: u128, lp_token_balance: u128) -> u32 {
    if lp_token_balance == 0 {
        return 0;
    }
    ((lp_tokens * u128::from(protocol::VOTE_WEIGHT_SCALE_FACTOR)) / lp_token_balance)
        .try_into()
        .unwrap_or(u32::MAX)
}

pub fn run_amm_vote_apply_facts(facts: &AMMVoteApplyFacts) -> AMMVoteApplyResult {
    let mut updated_vote_slots = Vec::new();
    let mut numerator = 0_u128;
    let mut denominator = 0_u128;
    let mut found_account = false;
    let mut min_tokens: Option<u128> = None;
    let mut min_pos = 0_usize;
    let mut min_account = protocol::AccountID::zero();
    let mut min_fee = 0_u16;

    for slot in facts.vote_slots.iter().copied() {
        if slot.lp_tokens == 0 {
            continue;
        }

        let (lp_tokens, fee) = if slot.account == facts.account {
            found_account = true;
            (facts.account_lp_tokens, facts.trading_fee)
        } else {
            (slot.lp_tokens, slot.trading_fee)
        };

        if lp_tokens == 0 {
            continue;
        }

        numerator += u128::from(fee) * lp_tokens;
        denominator += lp_tokens;

        if min_tokens.is_none()
            || lp_tokens < min_tokens.unwrap()
            || (lp_tokens == min_tokens.unwrap()
                && (fee < min_fee || (fee == min_fee && slot.account < min_account)))
        {
            min_tokens = Some(lp_tokens);
            min_pos = updated_vote_slots.len();
            min_account = slot.account;
            min_fee = fee;
        }

        updated_vote_slots.push(AMMVoteResultSlot {
            account: slot.account,
            trading_fee: (fee != 0).then_some(fee),
            vote_weight: vote_weight(lp_tokens, facts.lp_token_balance),
        });
    }

    if !found_account && facts.account_lp_tokens != 0 {
        let new_slot = AMMVoteResultSlot {
            account: facts.account,
            trading_fee: (facts.trading_fee != 0).then_some(facts.trading_fee),
            vote_weight: vote_weight(facts.account_lp_tokens, facts.lp_token_balance),
        };

        if updated_vote_slots.len() < usize::from(protocol::VOTE_MAX_SLOTS) {
            numerator += u128::from(facts.trading_fee) * facts.account_lp_tokens;
            denominator += facts.account_lp_tokens;
            updated_vote_slots.push(new_slot);
        } else if let Some(tokens) = min_tokens
            && (facts.account_lp_tokens > tokens
                || (facts.account_lp_tokens == tokens && facts.trading_fee > min_fee))
        {
            let replaced_fee = updated_vote_slots[min_pos].trading_fee.unwrap_or(0);
            numerator = numerator - u128::from(replaced_fee) * tokens
                + u128::from(facts.trading_fee) * facts.account_lp_tokens;
            denominator = denominator - tokens + facts.account_lp_tokens;
            updated_vote_slots[min_pos] = new_slot;
        }
    }

    let trading_fee = if denominator == 0 {
        None
    } else {
        let fee = (numerator / denominator).try_into().unwrap_or(u16::MAX);
        (fee != 0).then_some(fee)
    };
    let discounted_fee = trading_fee
        .map(|fee| fee / protocol::AUCTION_SLOT_DISCOUNTED_FEE_FRACTION as u16)
        .filter(|fee| *fee != 0);

    AMMVoteApplyResult {
        vote_slots: updated_vote_slots,
        trading_fee,
        discounted_fee,
    }
}

pub trait AMMVoteApplySink {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry>;
    fn update_amm_entry(&mut self, sle: protocol::STLedgerEntry);
}

pub fn run_amm_vote_do_apply<S: AMMVoteApplySink>(facts: AMMVoteApplyFacts, sink: &mut S) -> Ter {
    let Some(amm_sle) = sink.get_amm_entry(&facts.asset1, &facts.asset2) else {
        return Ter::TER_NO_AMM;
    };

    let result = run_amm_vote_apply_facts(&facts);
    let mut obj = amm_sle.clone_as_object();
    let mut slots = protocol::STArray::new(get_field_by_symbol("sfVoteSlots"));
    for slot in result.vote_slots {
        let mut entry = protocol::STObject::make_inner_object(get_field_by_symbol("sfVoteEntry"));
        entry.set_account_id(get_field_by_symbol("sfAccount"), slot.account);
        if let Some(fee) = slot.trading_fee {
            entry.set_field_u16(get_field_by_symbol("sfTradingFee"), fee);
        }
        entry.set_field_u32(get_field_by_symbol("sfVoteWeight"), slot.vote_weight);
        slots.push_back(entry);
    }
    obj.set_field_array(get_field_by_symbol("sfVoteSlots"), slots);

    if let Some(fee) = result.trading_fee {
        obj.set_field_u16(get_field_by_symbol("sfTradingFee"), fee);
    } else if obj.is_field_present(get_field_by_symbol("sfTradingFee")) {
        obj.make_field_absent(get_field_by_symbol("sfTradingFee"));
    }

    if obj.is_field_present(get_field_by_symbol("sfAuctionSlot")) {
        let mut auction_slot = obj
            .peek_field_object(get_field_by_symbol("sfAuctionSlot"))
            .clone();
        if let Some(discounted_fee) = result.discounted_fee {
            auction_slot.set_field_u16(get_field_by_symbol("sfDiscountedFee"), discounted_fee);
        } else if auction_slot.is_field_present(get_field_by_symbol("sfDiscountedFee")) {
            auction_slot.make_field_absent(get_field_by_symbol("sfDiscountedFee"));
        }
        obj.set_field_object(get_field_by_symbol("sfAuctionSlot"), auction_slot);
    }

    sink.update_amm_entry(protocol::STLedgerEntry::from_stobject(obj, *amm_sle.key()));

    Ter::TES_SUCCESS
}

pub fn run_amm_vote_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}

fn get_field_by_symbol(symbol: &str) -> &'static protocol::SField {
    protocol::get_field_by_symbol(symbol)
}
