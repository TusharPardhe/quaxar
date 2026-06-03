//! `AmmClawback` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

#[derive(Debug, Clone, Copy)]
pub struct AMMClawbackPreflightFacts {
    pub issuer_equals_holder: bool,
    pub asset_is_xrp: bool,
    pub claw_two_assets: bool,
    pub asset_issuer_matches_asset2_issuer: bool,
    pub asset_issuer_matches_account: bool,
    pub claw_amount_asset_matches_asset: Option<bool>,
    pub claw_amount_signum: Option<i32>,
}

pub fn run_amm_clawback_check_extra_features(
    amm_clawback_enabled: bool,
    mptokens_v2_enabled: bool,
    claw_amount_is_mpt: bool,
    asset_is_mpt: bool,
    asset2_is_mpt: bool,
) -> bool {
    amm_clawback_enabled
        && (mptokens_v2_enabled || !(claw_amount_is_mpt || asset_is_mpt || asset2_is_mpt))
}

pub fn run_amm_clawback_preflight_facts(facts: AMMClawbackPreflightFacts) -> NotTec {
    if facts.issuer_equals_holder {
        return Ter::TEM_MALFORMED;
    }

    if facts.asset_is_xrp {
        return Ter::TEM_MALFORMED;
    }

    if facts.claw_two_assets && !facts.asset_issuer_matches_asset2_issuer {
        return Ter::TEM_INVALID_FLAG;
    }

    if !facts.asset_issuer_matches_account {
        return Ter::TEM_MALFORMED;
    }

    if facts
        .claw_amount_asset_matches_asset
        .is_some_and(|matches| !matches)
    {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.claw_amount_signum.is_some_and(|signum| signum <= 0) {
        return Ter::TEM_BAD_AMOUNT;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy)]
pub struct AMMClawbackPreclaimFacts {
    pub issuer_exists: bool,
    pub holder_exists: bool,
    pub amm_exists: bool,
    pub mptokens_v2_enabled: bool,
    pub issuer_allows_trustline_clawback: bool,
    pub issuer_no_freeze: bool,
    pub asset_claw_allowed: bool,
    pub claw_two_assets: bool,
    pub asset2_claw_allowed: bool,
}

pub fn run_amm_clawback_preclaim_facts(facts: AMMClawbackPreclaimFacts) -> Ter {
    if !facts.issuer_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.holder_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if !facts.mptokens_v2_enabled
        && (!facts.issuer_allows_trustline_clawback || facts.issuer_no_freeze)
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.asset_claw_allowed {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.claw_two_assets && !facts.asset2_claw_allowed {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_clawback_preflight<Registry, Tx, Journal, ParentBatchId>(
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

pub fn run_amm_clawback_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub fn run_amm_clawback_do_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
