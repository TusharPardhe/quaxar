//! `Payment` transactor port from `xrpld/src/libxrpl/tx/transactors/payment/the reference source`.

use super::transactor_defaults::FULLY_CANONICAL_SIGNATURE_FLAG;
use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, TransactorPreflight2Facts, TxConsequences, Validity,
    run_transactor_preflight0, run_transactor_preflight1, run_transactor_preflight2,
};
use protocol::{
    AccountID, INNER_BATCH_TRANSACTION_FLAG, NotTec, STAmount, SeqProxy, Ter, feature_batch,
    is_tes_success,
};

pub const MAX_PATH_SIZE: usize = 6;
pub const MAX_PATH_LENGTH: usize = 8;

pub const TF_PARTIAL_PAYMENT: u32 = 0x0002_0000;
pub const TF_LIMIT_QUALITY: u32 = 0x0004_0000;
pub const TF_NO_DIRECT_RIPPLE: u32 = 0x0001_0000;

pub const PAYMENT_MPT_V1_FLAGS_MASK: u32 =
    !(FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG | TF_PARTIAL_PAYMENT);
pub const PAYMENT_FLAGS_MASK: u32 = !(FULLY_CANONICAL_SIGNATURE_FLAG
    | INNER_BATCH_TRANSACTION_FLAG
    | TF_PARTIAL_PAYMENT
    | TF_LIMIT_QUALITY
    | TF_NO_DIRECT_RIPPLE);

pub const fn run_payment_check_extra_features(
    credential_ids_present: bool,
    credentials_enabled: bool,
    domain_id_present: bool,
    permissioned_dex_enabled: bool,
) -> bool {
    if credential_ids_present && !credentials_enabled {
        return false;
    }
    if domain_id_present && !permissioned_dex_enabled {
        return false;
    }
    true
}

pub const fn run_payment_get_flags_mask(
    destination_amount_is_mpt: bool,
    mptokens_v2_enabled: bool,
) -> u32 {
    if destination_amount_is_mpt && !mptokens_v2_enabled {
        PAYMENT_MPT_V1_FLAGS_MASK
    } else {
        PAYMENT_FLAGS_MASK
    }
}

pub const fn run_payment_calculate_max_xrp_spend(
    send_max_xrp_drops: Option<u64>,
    amount_xrp_drops: Option<u64>,
) -> u64 {
    match send_max_xrp_drops {
        Some(spend) => spend,
        None => match amount_xrp_drops {
            Some(amount) => amount,
            None => 0,
        },
    }
}

pub const fn run_payment_make_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    send_max_xrp_drops: Option<u64>,
    amount_xrp_drops: Option<u64>,
) -> TxConsequences {
    TxConsequences::with_potential_spend(
        fee_drops,
        seq_proxy,
        run_payment_calculate_max_xrp_spend(send_max_xrp_drops, amount_xrp_drops),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentPreflightEvalFacts {
    pub tx_flags: u32,
    pub mptokens_v1_enabled: bool,
    pub mptokens_v2_enabled: bool,
    pub amount_is_mpt: bool,
    pub paths_present: bool,
    pub send_max_present: bool,
    pub send_max_asset_matches_amount: bool,
    pub send_max_is_mpt: bool,
    pub amount_is_legal_net: bool,
    pub max_source_is_legal_net: bool,
    pub destination_present: bool,
    pub max_source_positive: bool,
    pub amount_positive: bool,
    pub src_asset_bad: bool,
    pub dst_asset_bad: bool,
    pub src_asset_is_xrp: bool,
    pub dst_asset_is_xrp: bool,
    pub account_equals_destination: bool,
    pub src_dst_tokens_equal: bool,
    pub deliver_min_present: bool,
    pub deliver_min_is_legal_net: bool,
    pub deliver_min_is_positive: bool,
    pub deliver_min_asset_matches_amount: bool,
    pub deliver_min_not_greater_than_amount: bool,
}

pub fn run_payment_preflight_eval(
    facts: PaymentPreflightEvalFacts,
    run_preflight1: impl FnOnce() -> NotTec,
    check_credentials_fields: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> NotTec {
    let ret = run_preflight1();
    if !is_tes_success(ret) {
        return ret;
    }

    if !facts.mptokens_v1_enabled && facts.amount_is_mpt {
        return Ter::TEM_DISABLED;
    }

    if !facts.mptokens_v2_enabled && facts.amount_is_mpt && facts.paths_present {
        return Ter::TEM_MALFORMED;
    }

    if !facts.mptokens_v2_enabled
        && ((facts.amount_is_mpt && !facts.send_max_asset_matches_amount)
            || (!facts.amount_is_mpt && facts.send_max_is_mpt))
    {
        return Ter::TEM_MALFORMED;
    }

    let partial_payment_allowed = (facts.tx_flags & TF_PARTIAL_PAYMENT) != 0;
    let limit_quality = (facts.tx_flags & TF_LIMIT_QUALITY) != 0;
    let default_paths_allowed = (facts.tx_flags & TF_NO_DIRECT_RIPPLE) == 0;
    let xrp_direct = facts.src_asset_is_xrp && facts.dst_asset_is_xrp;

    if !facts.amount_is_legal_net || !facts.max_source_is_legal_net {
        return Ter::TEM_BAD_AMOUNT;
    }
    if !facts.destination_present {
        return Ter::TEM_DST_NEEDED;
    }
    if facts.send_max_present && !facts.max_source_positive {
        return Ter::TEM_BAD_AMOUNT;
    }
    if !facts.amount_positive {
        return Ter::TEM_BAD_AMOUNT;
    }
    if facts.src_asset_bad || facts.dst_asset_bad {
        return Ter::TEM_BAD_CURRENCY;
    }
    if facts.account_equals_destination && facts.src_dst_tokens_equal && !facts.paths_present {
        return Ter::TEM_REDUNDANT;
    }
    if xrp_direct && facts.send_max_present {
        return Ter::TEM_BAD_SEND_XRP_MAX;
    }
    if (xrp_direct || (!facts.mptokens_v2_enabled && facts.amount_is_mpt)) && facts.paths_present {
        return Ter::TEM_BAD_SEND_XRP_PATHS;
    }
    if xrp_direct && partial_payment_allowed {
        return Ter::TEM_BAD_SEND_XRP_PARTIAL;
    }
    if (xrp_direct || (!facts.mptokens_v2_enabled && facts.amount_is_mpt)) && limit_quality {
        return Ter::TEM_BAD_SEND_XRP_LIMIT;
    }
    if (xrp_direct || (!facts.mptokens_v2_enabled && facts.amount_is_mpt)) && !default_paths_allowed
    {
        return Ter::TEM_BAD_SEND_XRP_NO_DIRECT;
    }

    if facts.deliver_min_present {
        if !partial_payment_allowed {
            return Ter::TEM_BAD_AMOUNT;
        }
        if !facts.deliver_min_is_legal_net || !facts.deliver_min_is_positive {
            return Ter::TEM_BAD_AMOUNT;
        }
        if !facts.deliver_min_asset_matches_amount {
            return Ter::TEM_BAD_AMOUNT;
        }
        if !facts.deliver_min_not_greater_than_amount {
            return Ter::TEM_BAD_AMOUNT;
        }
    }

    let ret = check_credentials_fields();
    if !is_tes_success(ret) {
        return ret;
    }

    run_preflight2()
}

pub struct PaymentPreflightFacts {
    pub amount: STAmount,
    pub deliver_min: Option<STAmount>,
    pub destination: AccountID,
    pub flags: u32,
    pub domain_id: Option<basics::base_uint::Uint256>,
    pub fix_cleanup_3_2_0: bool,
}

pub fn run_payment_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: PaymentPreflightFacts,
) -> NotTec {
    // A zero DomainID is invalid for a PermissionedDomain ledger entry because
    // keylet::permissionedDomain(uint256) uses the DomainID as the ledger key.
    if facts.fix_cleanup_3_2_0 {
        if let Some(ref domain_id) = facts.domain_id {
            if domain_id.is_zero() {
                return Ter::TEM_MALFORMED;
            }
        }
    }

    let deliver_min = facts.deliver_min;
    let eval_facts = PaymentPreflightEvalFacts {
        tx_flags: facts.flags,
        mptokens_v1_enabled: true,
        mptokens_v2_enabled: true,
        amount_is_mpt: facts.amount.holds_mpt_issue(),
        paths_present: false,
        send_max_present: false,
        send_max_asset_matches_amount: true,
        send_max_is_mpt: facts.amount.holds_mpt_issue(),
        amount_is_legal_net: facts.amount.is_legal_net(),
        max_source_is_legal_net: facts.amount.is_legal_net(),
        destination_present: !facts.destination.is_zero(),
        max_source_positive: facts.amount.signum() > 0,
        amount_positive: facts.amount.signum() > 0,
        src_asset_bad: false,
        dst_asset_bad: false,
        src_asset_is_xrp: facts.amount.native(),
        dst_asset_is_xrp: facts.amount.native(),
        account_equals_destination: false,
        src_dst_tokens_equal: true,
        deliver_min_present: deliver_min.is_some(),
        deliver_min_is_legal_net: deliver_min.as_ref().is_none_or(STAmount::is_legal_net),
        deliver_min_is_positive: deliver_min.as_ref().is_none_or(|value| value.signum() > 0),
        deliver_min_asset_matches_amount: deliver_min
            .as_ref()
            .is_none_or(|value| value.asset() == facts.amount.asset()),
        deliver_min_not_greater_than_amount: deliver_min
            .as_ref()
            .is_none_or(|value| value <= &facts.amount),
    };

    run_payment_preflight_eval(
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
                        run_payment_get_flags_mask(facts.amount.holds_mpt_issue(), true),
                    )
                },
                || Ter::TES_SUCCESS,
            )
        },
        || Ter::TES_SUCCESS,
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
pub struct PaymentCheckPermissionFacts {
    pub delegate_present: bool,
    pub delegate_entry_exists: bool,
    pub check_tx_permission_result: NotTec,
    pub send_max_present: bool,
    pub send_max_asset_matches_amount: bool,
    pub paths_present: bool,
    pub payment_mint_permission: bool,
    pub payment_burn_permission: bool,
    pub amount_is_xrp: bool,
    pub amount_issuer_is_source: bool,
    pub amount_issuer_is_destination: bool,
}

pub fn run_payment_check_permission(facts: PaymentCheckPermissionFacts) -> NotTec {
    if !facts.delegate_present {
        return Ter::TES_SUCCESS;
    }
    if !facts.delegate_entry_exists {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if is_tes_success(facts.check_tx_permission_result) {
        return Ter::TES_SUCCESS;
    }
    if (facts.send_max_present && !facts.send_max_asset_matches_amount) || facts.paths_present {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if facts.payment_mint_permission && !facts.amount_is_xrp && facts.amount_issuer_is_source {
        return Ter::TES_SUCCESS;
    }
    if facts.payment_burn_permission && !facts.amount_is_xrp && facts.amount_issuer_is_destination {
        return Ter::TES_SUCCESS;
    }
    Ter::TER_NO_DELEGATE_PERMISSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentPreclaimFacts {
    pub tx_flags: u32,
    pub has_paths: bool,
    pub send_max_present: bool,
    pub dst_amount_native: bool,
    pub destination_exists: bool,
    pub view_open: bool,
    pub destination_requires_tag: bool,
    pub destination_tag_present: bool,
    pub destination_can_create_with_amount: bool,
    pub path_count: usize,
    pub path_has_too_long_segment: bool,
    pub credentials_valid_result: Ter,
    pub domain_id_present: bool,
    pub source_in_domain: bool,
    pub destination_in_domain: bool,
    pub is_batch_inner: bool,
    pub batch_v1_1_enabled: bool,
}

impl PaymentPreclaimFacts {
    pub const fn success_defaults() -> Self {
        Self {
            tx_flags: 0,
            has_paths: false,
            send_max_present: false,
            dst_amount_native: true,
            destination_exists: true,
            view_open: false,
            destination_requires_tag: false,
            destination_tag_present: false,
            destination_can_create_with_amount: true,
            path_count: 0,
            path_has_too_long_segment: false,
            credentials_valid_result: Ter::TES_SUCCESS,
            domain_id_present: false,
            source_in_domain: true,
            destination_in_domain: true,
            is_batch_inner: false,
            batch_v1_1_enabled: false,
        }
    }
}

pub fn run_payment_preclaim_with_facts(facts: PaymentPreclaimFacts) -> Ter {
    let partial_payment_allowed = (facts.tx_flags & TF_PARTIAL_PAYMENT) != 0;
    if !facts.destination_exists {
        if !facts.dst_amount_native {
            return Ter::TEC_NO_DST;
        }
        if facts.view_open && partial_payment_allowed {
            return Ter::TEL_NO_DST_PARTIAL;
        }
        if facts.is_batch_inner && facts.batch_v1_1_enabled && partial_payment_allowed {
            return Ter::TEF_NO_DST_PARTIAL;
        }
        if !facts.destination_can_create_with_amount {
            return Ter::TEC_NO_DST_INSUF_XRP;
        }
    } else if facts.destination_requires_tag && !facts.destination_tag_present {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    if facts.has_paths || facts.send_max_present || !facts.dst_amount_native {
        if facts.path_count > MAX_PATH_SIZE || facts.path_has_too_long_segment {
            if facts.view_open {
                return Ter::TEL_BAD_PATH_COUNT;
            }
            if facts.is_batch_inner && facts.batch_v1_1_enabled {
                return Ter::TEF_BAD_PATH_COUNT;
            }
        }
    }

    if !is_tes_success(facts.credentials_valid_result) {
        return facts.credentials_valid_result;
    }
    if facts.domain_id_present && (!facts.source_in_domain || !facts.destination_in_domain) {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentDoApplyBranch {
    Ripple,
    MptDirect,
    XrpDirect,
}

pub const fn run_payment_select_do_apply_branch(
    has_paths: bool,
    send_max_present: bool,
    dst_amount_native: bool,
    dst_amount_is_mpt: bool,
    mptokens_v2_enabled: bool,
) -> PaymentDoApplyBranch {
    if (has_paths || send_max_present || !dst_amount_native)
        && (!dst_amount_is_mpt || mptokens_v2_enabled)
    {
        PaymentDoApplyBranch::Ripple
    } else if dst_amount_is_mpt {
        PaymentDoApplyBranch::MptDirect
    } else {
        PaymentDoApplyBranch::XrpDirect
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentDoApplyFacts {
    pub has_paths: bool,
    pub send_max_present: bool,
    pub dst_amount_native: bool,
    pub dst_amount_is_mpt: bool,
    pub mptokens_v2_enabled: bool,
    pub ripple_verify_deposit_preauth_result: Ter,
    pub ripple_calc_result: Ter,
    pub ripple_actual_out_equals_dst: bool,
    pub ripple_actual_out_below_deliver_min: bool,
    pub ripple_result_is_retry: bool,
    pub mpt_require_auth_source_result: Ter,
    pub mpt_require_auth_destination_result: Ter,
    pub mpt_can_transfer_result: Ter,
    pub mpt_verify_deposit_preauth_result: Ter,
    pub mpt_holder_to_holder: bool,
    pub mpt_any_frozen: bool,
    pub mpt_destination_token_exists: bool,
    pub mpt_destination_reserve_satisfied: bool,
    pub mpt_issuance_exists: bool,
    pub mpt_required_exceeds_max_source_after_partial_adjust: bool,
    pub mpt_deliver_below_deliver_min: bool,
    pub mpt_account_send_result: Ter,
    pub mpt_fix_delivered_amount_enabled: bool,
    pub mpt_delivered_differs_from_dst: bool,
    pub mpt_transfer_fee: u16,
    pub mpt_maximum_amount: Option<u64>,
    pub mpt_outstanding_amount: u64,
    pub mpt_amount: u64,
    pub mpt_issuer_is_source: bool,
    pub mpt_issuer_is_destination: bool,
    pub xrp_source_exists: bool,
    pub xrp_has_funds_for_payment_plus_reserve: bool,
    pub xrp_destination_is_pseudo: bool,
    pub xrp_needs_deposit_preauth: bool,
    pub xrp_verify_deposit_preauth_result: Ter,
    pub xrp_destination_password_spent: bool,
}

impl PaymentDoApplyFacts {
    pub const fn xrp_success_defaults() -> Self {
        Self {
            has_paths: false,
            send_max_present: false,
            dst_amount_native: true,
            dst_amount_is_mpt: false,
            mptokens_v2_enabled: false,
            ripple_verify_deposit_preauth_result: Ter::TES_SUCCESS,
            ripple_calc_result: Ter::TES_SUCCESS,
            ripple_actual_out_equals_dst: true,
            ripple_actual_out_below_deliver_min: false,
            ripple_result_is_retry: false,
            mpt_require_auth_source_result: Ter::TES_SUCCESS,
            mpt_require_auth_destination_result: Ter::TES_SUCCESS,
            mpt_can_transfer_result: Ter::TES_SUCCESS,
            mpt_verify_deposit_preauth_result: Ter::TES_SUCCESS,
            mpt_holder_to_holder: false,
            mpt_any_frozen: false,
            mpt_destination_token_exists: true,
            mpt_destination_reserve_satisfied: true,
            mpt_issuance_exists: true,
            mpt_required_exceeds_max_source_after_partial_adjust: false,
            mpt_deliver_below_deliver_min: false,
            mpt_account_send_result: Ter::TES_SUCCESS,
            mpt_fix_delivered_amount_enabled: false,
            mpt_delivered_differs_from_dst: false,
            mpt_transfer_fee: 0,
            mpt_maximum_amount: None,
            mpt_outstanding_amount: 0,
            mpt_amount: 0,
            mpt_issuer_is_source: false,
            mpt_issuer_is_destination: false,
            xrp_source_exists: true,
            xrp_has_funds_for_payment_plus_reserve: true,
            xrp_destination_is_pseudo: false,
            xrp_needs_deposit_preauth: false,
            xrp_verify_deposit_preauth_result: Ter::TES_SUCCESS,
            xrp_destination_password_spent: false,
        }
    }
}

pub trait PaymentDoApplySink {
    fn touch_destination(&mut self) {}
    fn apply_ripple_payment(&mut self) {}
    fn record_ripple_delivered_amount(&mut self) {}
    fn apply_mpt_payment(&mut self) {}
    fn record_mpt_delivered_amount(&mut self) {}
    fn apply_xrp_payment(&mut self) {}
    fn clear_xrp_password_spent(&mut self) {}
}

pub fn run_payment_do_apply_with_facts<S: PaymentDoApplySink>(
    facts: PaymentDoApplyFacts,
    sink: &mut S,
) -> Ter {
    sink.touch_destination();

    match run_payment_select_do_apply_branch(
        facts.has_paths,
        facts.send_max_present,
        facts.dst_amount_native,
        facts.dst_amount_is_mpt,
        facts.mptokens_v2_enabled,
    ) {
        PaymentDoApplyBranch::Ripple => {
            if !is_tes_success(facts.ripple_verify_deposit_preauth_result) {
                return facts.ripple_verify_deposit_preauth_result;
            }
            sink.apply_ripple_payment();

            let mut result = facts.ripple_calc_result;
            if is_tes_success(result) && !facts.ripple_actual_out_equals_dst {
                if facts.ripple_actual_out_below_deliver_min {
                    result = Ter::TEC_PATH_PARTIAL;
                } else {
                    sink.record_ripple_delivered_amount();
                }
            }
            if facts.ripple_result_is_retry {
                Ter::TEC_PATH_DRY
            } else {
                result
            }
        }
        PaymentDoApplyBranch::MptDirect => {
            if !facts.mpt_issuance_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            if !is_tes_success(facts.mpt_require_auth_source_result) {
                return facts.mpt_require_auth_source_result;
            }
            if !is_tes_success(facts.mpt_require_auth_destination_result) {
                return facts.mpt_require_auth_destination_result;
            }
            if !is_tes_success(facts.mpt_can_transfer_result) {
                return facts.mpt_can_transfer_result;
            }
            if !is_tes_success(facts.mpt_verify_deposit_preauth_result) {
                return facts.mpt_verify_deposit_preauth_result;
            }
            if facts.mpt_any_frozen {
                return Ter::TEC_LOCKED;
            }

            if !facts.mpt_destination_token_exists && !facts.mpt_destination_reserve_satisfied {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }

            if let Some(max_amt) = facts.mpt_maximum_amount {
                if facts.mpt_outstanding_amount + facts.mpt_amount > max_amt {
                    return Ter::TEC_PATH_PARTIAL;
                }
            }

            if facts.mpt_required_exceeds_max_source_after_partial_adjust
                || facts.mpt_deliver_below_deliver_min
            {
                return Ter::TEC_PATH_PARTIAL;
            }

            let mut result = facts.mpt_account_send_result;
            if is_tes_success(result) {
                sink.apply_mpt_payment();
                if facts.mpt_fix_delivered_amount_enabled && facts.mpt_delivered_differs_from_dst {
                    sink.record_mpt_delivered_amount();
                }
            } else if result == Ter::TEC_INSUFFICIENT_FUNDS || result == Ter::TEC_PATH_DRY {
                result = Ter::TEC_PATH_PARTIAL;
            }
            result
        }
        PaymentDoApplyBranch::XrpDirect => {
            if !facts.xrp_source_exists {
                return Ter::TEF_INTERNAL;
            }
            if !facts.xrp_has_funds_for_payment_plus_reserve {
                return Ter::TEC_UNFUNDED_PAYMENT;
            }
            if facts.xrp_destination_is_pseudo {
                return Ter::TEC_NO_PERMISSION;
            }
            if facts.xrp_needs_deposit_preauth
                && !is_tes_success(facts.xrp_verify_deposit_preauth_result)
            {
                return facts.xrp_verify_deposit_preauth_result;
            }
            sink.apply_xrp_payment();
            if facts.xrp_destination_password_spent {
                sink.clear_xrp_password_spent();
            }
            Ter::TES_SUCCESS
        }
    }
}

pub fn run_payment_do_apply_result_with_facts<S: PaymentDoApplySink>(
    facts: PaymentDoApplyFacts,
    sink: &mut S,
) -> ApplyResult {
    let ter = run_payment_do_apply_with_facts(facts, sink);
    ApplyResult::new(ter, is_tes_success(ter), false)
}

pub fn run_payment_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    // Runtime wrapper stays narrow until full ledger-view state is wired.
    run_payment_preclaim_with_facts(PaymentPreclaimFacts::success_defaults())
}

struct NoopPaymentDoApplySink;
impl PaymentDoApplySink for NoopPaymentDoApplySink {}

pub fn run_payment_do_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    let mut sink = NoopPaymentDoApplySink;
    run_payment_do_apply_result_with_facts(PaymentDoApplyFacts::xrp_success_defaults(), &mut sink)
}
