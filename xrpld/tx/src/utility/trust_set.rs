//! `TrustSet` transactor port from `xrpld/src/libxrpl/tx/transactors/token/the reference source`.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, TransactorPreflight2Facts, Validity, run_transactor_preflight0,
    run_transactor_preflight1, run_transactor_preflight2,
};
use protocol::{
    NotTec, QUALITY_ONE, STAmount, Ter, bad_currency, feature_batch, is_tes_success,
    lsfDisallowIncomingTrustline, lsfHighDeepFreeze, lsfHighFreeze, lsfLowDeepFreeze, lsfLowFreeze,
    tfClearDeepFreeze, tfClearFreeze, tfSetDeepFreeze, tfSetFreeze, tfSetfAuth, tfTrustSetMask,
    tfTrustSetPermissionMask,
};

pub const fn run_trust_set_get_flags_mask() -> u32 {
    tfTrustSetMask
}

pub const fn run_trust_set_compute_freeze_flags(
    mut flags: u32,
    high_side: bool,
    no_freeze: bool,
    set_freeze: bool,
    clear_freeze: bool,
    set_deep_freeze: bool,
    clear_deep_freeze: bool,
) -> u32 {
    if set_freeze && !clear_freeze && !no_freeze {
        flags |= if high_side {
            lsfHighFreeze
        } else {
            lsfLowFreeze
        };
    } else if clear_freeze && !set_freeze {
        flags &= !(if high_side {
            lsfHighFreeze
        } else {
            lsfLowFreeze
        });
    }

    if set_deep_freeze && !clear_deep_freeze && !no_freeze {
        flags |= if high_side {
            lsfHighDeepFreeze
        } else {
            lsfLowDeepFreeze
        };
    } else if clear_deep_freeze && !set_deep_freeze {
        flags &= !(if high_side {
            lsfHighDeepFreeze
        } else {
            lsfLowDeepFreeze
        });
    }

    flags
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustSetPreflightEvalFacts {
    pub tx_flags: u32,
    pub deep_freeze_enabled: bool,
    pub limit_is_legal_net: bool,
    pub limit_is_native: bool,
    pub limit_currency_is_bad: bool,
    pub limit_is_negative: bool,
    pub issuer_present: bool,
}

pub fn run_trust_set_preflight_eval(
    facts: TrustSetPreflightEvalFacts,
    run_preflight1: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> NotTec {
    let ret = run_preflight1();
    if !is_tes_success(ret) {
        return ret;
    }

    if !facts.deep_freeze_enabled && (facts.tx_flags & (tfSetDeepFreeze | tfClearDeepFreeze)) != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    if !facts.limit_is_legal_net {
        return Ter::TEM_BAD_AMOUNT;
    }
    if facts.limit_is_native {
        return Ter::TEM_BAD_LIMIT;
    }
    if facts.limit_currency_is_bad {
        return Ter::TEM_BAD_CURRENCY;
    }
    if facts.limit_is_negative {
        return Ter::TEM_BAD_LIMIT;
    }
    if !facts.issuer_present {
        return Ter::TEM_DST_NEEDED;
    }

    run_preflight2()
}

pub struct TrustSetPreflightFacts {
    pub limit_amount: STAmount,
    pub quality_in: Option<u32>,
    pub quality_out: Option<u32>,
    pub flags: u32,
    pub deep_freeze_enabled: bool,
}

pub fn run_trust_set_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: TrustSetPreflightFacts,
) -> NotTec {
    let eval_facts = TrustSetPreflightEvalFacts {
        tx_flags: facts.flags,
        deep_freeze_enabled: facts.deep_freeze_enabled,
        limit_is_legal_net: facts.limit_amount.is_legal_net(),
        limit_is_native: facts.limit_amount.native(),
        limit_currency_is_bad: !facts.limit_amount.native()
            && facts.limit_amount.holds_issue()
            && facts.limit_amount.issue().currency == bad_currency(),
        limit_is_negative: facts.limit_amount.signum() < 0,
        issuer_present: !facts.limit_amount.native()
            && facts.limit_amount.holds_issue()
            && !facts.limit_amount.issue().account.is_zero(),
    };

    run_trust_set_preflight_eval(
        eval_facts,
        || {
            run_transactor_preflight1(
                TransactorPreflight1Facts {
                    inner_batch_flag_set: (ctx.flags.bits() & crate::ApplyFlags::BATCH.bits()) != 0,
                    batch_enabled: ctx.rules.enabled(&feature_batch()),
                    ..Default::default()
                },
                || {
                    run_transactor_preflight0(
                        TransactorPreflight0Facts {
                            tx_flags: facts.flags,
                            ..Default::default()
                        },
                        run_trust_set_get_flags_mask(),
                    )
                },
                || Ter::TES_SUCCESS,
            )
        },
        || {
            run_transactor_preflight2(
                TransactorPreflight2Facts {
                    ..Default::default()
                },
                || None,
                || Validity::Valid,
            )
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustSetCheckPermissionFacts {
    pub delegate_present: bool,
    pub delegate_entry_exists: bool,
    pub check_tx_permission_result: NotTec,
    pub tx_flags: u32,
    pub quality_in_present: bool,
    pub quality_out_present: bool,
    pub trustline_exists: bool,
    pub granular_trustline_authorize: bool,
    pub granular_trustline_freeze: bool,
    pub granular_trustline_unfreeze: bool,
    pub current_limit_equals_proposed_limit: bool,
}

pub fn run_trust_set_check_permission(facts: TrustSetCheckPermissionFacts) -> NotTec {
    if !facts.delegate_present {
        return Ter::TES_SUCCESS;
    }
    if !facts.delegate_entry_exists {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if is_tes_success(facts.check_tx_permission_result) {
        return Ter::TES_SUCCESS;
    }
    if (facts.tx_flags & tfTrustSetPermissionMask) != 0 {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if facts.quality_in_present || facts.quality_out_present {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if !facts.trustline_exists {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if (facts.tx_flags & tfSetfAuth) != 0 && !facts.granular_trustline_authorize {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if (facts.tx_flags & tfSetFreeze) != 0 && !facts.granular_trustline_freeze {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if (facts.tx_flags & tfClearFreeze) != 0 && !facts.granular_trustline_unfreeze {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if !facts.current_limit_equals_proposed_limit {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustSetPreclaimFacts {
    pub account_exists: bool,
    pub tx_flags: u32,
    pub account_requires_auth: bool,
    pub destination_is_source: bool,
    pub destination_exists: bool,
    pub amm_or_single_asset_vault_enabled: bool,
    pub destination_account_flags: u32,
    pub fix_disallow_incoming_v1_enabled: bool,
    pub trustline_exists: bool,
    pub destination_is_pseudo_account: bool,
    pub pseudo_destination_is_amm: bool,
    pub pseudo_destination_is_vault_or_loan_broker: bool,
    pub amm_ledger_entry_exists: bool,
    pub amm_lp_token_balance_non_zero: bool,
    pub amm_lp_token_currency_matches_limit: bool,
    pub deep_freeze_enabled: bool,
    pub account_no_freeze: bool,
    pub high_account_side: bool,
    pub current_trustline_flags: u32,
}

pub fn run_trust_set_preclaim_with_facts(facts: TrustSetPreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    let set_auth = (facts.tx_flags & tfSetfAuth) != 0;
    let set_freeze = (facts.tx_flags & tfSetFreeze) != 0;
    let clear_freeze = (facts.tx_flags & tfClearFreeze) != 0;
    let set_deep_freeze = (facts.tx_flags & tfSetDeepFreeze) != 0;
    let clear_deep_freeze = (facts.tx_flags & tfClearDeepFreeze) != 0;

    if set_auth && !facts.account_requires_auth {
        return Ter::TEF_NO_AUTH_REQUIRED;
    }
    if facts.destination_is_source {
        return Ter::TEM_DST_IS_SRC;
    }
    if facts.amm_or_single_asset_vault_enabled && !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if (facts.destination_account_flags & lsfDisallowIncomingTrustline) != 0 {
        if !(facts.fix_disallow_incoming_v1_enabled && facts.trustline_exists) {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    if facts.destination_is_pseudo_account {
        if facts.pseudo_destination_is_amm {
            if !facts.trustline_exists {
                if !facts.amm_ledger_entry_exists {
                    return Ter::TEC_INTERNAL;
                }
                if !facts.amm_lp_token_balance_non_zero {
                    return Ter::TEC_AMM_EMPTY;
                }
                if !facts.amm_lp_token_currency_matches_limit {
                    return Ter::TEC_NO_PERMISSION;
                }
            }
        } else if facts.pseudo_destination_is_vault_or_loan_broker {
            if !facts.trustline_exists {
                return Ter::TEC_NO_PERMISSION;
            }
        } else {
            return Ter::TEC_PSEUDO_ACCOUNT;
        }
    }

    if facts.deep_freeze_enabled {
        if facts.account_no_freeze && (set_freeze || set_deep_freeze) {
            return Ter::TEC_NO_PERMISSION;
        }
        if (set_freeze || set_deep_freeze) && (clear_freeze || clear_deep_freeze) {
            return Ter::TEC_NO_PERMISSION;
        }

        let expected_flags = run_trust_set_compute_freeze_flags(
            facts.current_trustline_flags,
            facts.high_account_side,
            facts.account_no_freeze,
            set_freeze,
            clear_freeze,
            set_deep_freeze,
            clear_deep_freeze,
        );
        let frozen_mask = if facts.high_account_side {
            lsfHighFreeze
        } else {
            lsfLowFreeze
        };
        let deep_frozen_mask = if facts.high_account_side {
            lsfHighDeepFreeze
        } else {
            lsfLowDeepFreeze
        };

        if (expected_flags & deep_frozen_mask) != 0 && (expected_flags & frozen_mask) == 0 {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_trust_set_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    // Runtime wrapper stays narrow until full ledger-view state is wired.
    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustSetDoApplyFacts {
    pub source_account_exists: bool,
    pub destination_account_exists: bool,
    pub line_exists: bool,
    pub limit_is_zero: bool,
    pub quality_in_present: bool,
    pub quality_in: u32,
    pub quality_out_present: bool,
    pub quality_out: u32,
    pub set_auth: bool,
    pub set_no_ripple: bool,
    pub clear_no_ripple: bool,
    pub set_no_ripple_on_negative_balance_attempt: bool,
    pub default_after_update: bool,
    pub currency_is_bad: bool,
    pub reserve_increase_for_local_side: bool,
    pub local_has_reserve_for_increase: bool,
    pub has_reserve_to_create_line: bool,
    pub trust_delete_result: Ter,
    pub trust_create_result: Ter,
}

impl TrustSetDoApplyFacts {
    pub const fn success_defaults() -> Self {
        Self {
            source_account_exists: true,
            destination_account_exists: true,
            line_exists: true,
            limit_is_zero: false,
            quality_in_present: false,
            quality_in: 0,
            quality_out_present: false,
            quality_out: 0,
            set_auth: false,
            set_no_ripple: false,
            clear_no_ripple: false,
            set_no_ripple_on_negative_balance_attempt: false,
            default_after_update: false,
            currency_is_bad: false,
            reserve_increase_for_local_side: false,
            local_has_reserve_for_increase: true,
            has_reserve_to_create_line: true,
            trust_delete_result: Ter::TES_SUCCESS,
            trust_create_result: Ter::TES_SUCCESS,
        }
    }
}

pub trait TrustSetDoApplySink {
    fn update_existing_line(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn delete_existing_line(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
    fn create_line(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
}

pub fn run_trust_set_do_apply_with_facts<S: TrustSetDoApplySink>(
    facts: TrustSetDoApplyFacts,
    sink: &mut S,
) -> Ter {
    if !facts.source_account_exists {
        return Ter::TEF_INTERNAL;
    }
    if !facts.destination_account_exists {
        return Ter::TEC_NO_DST;
    }

    let quality_in = if facts.quality_in_present && facts.quality_in != QUALITY_ONE {
        facts.quality_in
    } else {
        0
    };
    let quality_out = if facts.quality_out_present && facts.quality_out != QUALITY_ONE {
        facts.quality_out
    } else {
        0
    };

    if facts.line_exists {
        if facts.set_no_ripple
            && !facts.clear_no_ripple
            && facts.set_no_ripple_on_negative_balance_attempt
        {
            return Ter::TEC_NO_PERMISSION;
        }

        if facts.default_after_update || facts.currency_is_bad {
            let ter = sink.delete_existing_line();
            return if is_tes_success(ter) {
                facts.trust_delete_result
            } else {
                ter
            };
        }

        if facts.reserve_increase_for_local_side && !facts.local_has_reserve_for_increase {
            return Ter::TEC_INSUF_RESERVE_LINE;
        }

        let ter = sink.update_existing_line();
        return if is_tes_success(ter) {
            Ter::TES_SUCCESS
        } else {
            ter
        };
    }

    let default_quality_in = !facts.quality_in_present || quality_in == 0;
    let default_quality_out = !facts.quality_out_present || quality_out == 0;

    if facts.limit_is_zero && default_quality_in && default_quality_out && !facts.set_auth {
        return Ter::TEC_NO_LINE_REDUNDANT;
    }
    if !facts.has_reserve_to_create_line {
        return Ter::TEC_NO_LINE_INSUF_RESERVE;
    }

    let ter = sink.create_line();
    if is_tes_success(ter) {
        facts.trust_create_result
    } else {
        ter
    }
}

pub fn run_trust_set_do_apply_result_with_facts<S: TrustSetDoApplySink>(
    facts: TrustSetDoApplyFacts,
    sink: &mut S,
) -> ApplyResult {
    let ter = run_trust_set_do_apply_with_facts(facts, sink);
    ApplyResult::new(ter, is_tes_success(ter), false)
}

struct NoopTrustSetApplySink;
impl TrustSetDoApplySink for NoopTrustSetApplySink {}

pub fn run_trust_set_do_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    let mut sink = NoopTrustSetApplySink;
    run_trust_set_do_apply_result_with_facts(TrustSetDoApplyFacts::success_defaults(), &mut sink)
}
