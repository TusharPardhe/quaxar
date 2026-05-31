//! the reference implementation compatibility surface.
//!
//! This ports the exact current deterministic `preflight(...)`,
//! `preclaim(...)`, and staged `doApply()` shells.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashPreflightFacts {
    pub amount_present: bool,
    pub deliver_min_present: bool,
    pub value_is_legal: bool,
    pub value_signum_positive: bool,
    pub value_currency_is_bad: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashPreclaimFacts {
    pub check_exists: bool,
    pub tx_account_is_check_destination: bool,
    pub check_source_is_destination: bool,
    pub source_account_exists: bool,
    pub destination_account_exists: bool,
    pub destination_require_dest_tag: bool,
    pub check_has_destination_tag: bool,
    pub check_expired: bool,
    pub requested_currency_matches_send_max: bool,
    pub requested_issuer_matches_send_max: bool,
    pub requested_value_exceeds_send_max: bool,
    pub requested_value_exceeds_available_funds: bool,
    pub requested_value_native: bool,
    pub requested_value_issuer_is_destination: bool,
    pub issuer_exists: bool,
    pub issuer_requires_auth: bool,
    pub destination_trustline_exists: bool,
    pub destination_trustline_authorized: bool,
    pub destination_trustline_frozen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashApplyFacts {
    pub check_exists: bool,
    pub source_account_exists: bool,
    pub destination_account_exists: bool,
    pub source_is_destination: bool,
    pub send_max_is_native: bool,
    pub amount_present: bool,
    pub deliver_min_present: bool,
    pub iou_trustline_exists: bool,
    pub iou_destination_can_pay_trustline_reserve: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouFlowResult {
    pub ter: Ter,
    pub meets_requested_amount: bool,
    pub meets_deliver_min: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckCashIouLimitSide {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustlinePlan<AccountId> {
    pub truster: AccountId,
    pub needs_create: bool,
    pub tweaked_limit_side: CheckCashIouLimitSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustCreatePlan<Balance, Limit> {
    pub needs_create: bool,
    pub can_create: bool,
    pub dest_low: bool,
    pub authorize_account: bool,
    pub no_ripple: bool,
    pub freeze: bool,
    pub deep_freeze: bool,
    pub initial_balance: Balance,
    pub limit: Limit,
    pub quality_in: u32,
    pub quality_out: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustCreateFieldShape {
    pub dest_low: bool,
    pub initial_balance_is_zero: bool,
    pub destination_limit_is_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustCreateDestinationState {
    pub owner_count: u32,
    pub default_ripple_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustCreateHandoffPlan<
    AccountId,
    TrustLineKey,
    TrustLineIndex,
    Balance,
    Limit,
> {
    pub truster: AccountId,
    pub trustline_key: TrustLineKey,
    pub trustline_index: TrustLineIndex,
    pub destination_update_after_create: bool,
    pub needs_create: bool,
    pub can_create: bool,
    pub dest_low: bool,
    pub authorize_account: bool,
    pub no_ripple: bool,
    pub freeze: bool,
    pub deep_freeze: bool,
    pub initial_balance: Balance,
    pub limit: Limit,
    pub quality_in: u32,
    pub quality_out: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouTrustCreateCallerPlan<
    AccountId,
    TrustLineKey,
    TrustLineIndex,
    Balance,
    Limit,
> {
    pub destination_state: CheckCashIouTrustCreateDestinationState,
    pub trust_create:
        CheckCashIouTrustCreateHandoffPlan<AccountId, TrustLineKey, TrustLineIndex, Balance, Limit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouLimitOverridePlan<T> {
    pub tweaked_limit_side: CheckCashIouLimitSide,
    pub temporary_limit: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouFlowRequest<T> {
    pub deliver: T,
    pub partial_payment: bool,
    pub default_path: bool,
    pub owner_pays_transfer_fee: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouFlowCallPlan<T> {
    pub request: CheckCashIouFlowRequest<T>,
    pub limit_override: CheckCashIouLimitOverridePlan<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashIouFlowOutcome {
    pub ter: Ter,
    pub record_delivered_amount: bool,
    pub reload_check_after_flow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckCashDoApplyBranch {
    SelfCash,
    Xrp,
    Iou,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashDoApplyCleanupPlan {
    pub remove_destination_dir: bool,
    pub remove_owner_dir: bool,
    pub adjust_owner_count_delta: i32,
    pub erase_check: bool,
    pub apply_view: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCashDoApplyPlan {
    pub branch: CheckCashDoApplyBranch,
    pub cleanup: CheckCashDoApplyCleanupPlan,
}

pub trait CheckCashApplySink {
    fn xrp_liquid_sufficient(&mut self) -> bool;
    fn record_delivered_xrp(&mut self);
    fn transfer_xrp(&mut self) -> Ter;
    fn create_iou_trustline(&mut self) -> Ter;
    fn update_destination_after_trustline_create(&mut self);
    fn trustline_available_after_create(&mut self) -> bool {
        true
    }
    fn prepare_iou_flow_limit(&mut self) -> Ter;
    fn run_iou_flow(&mut self, deliver_min_present: bool) -> CheckCashIouFlowResult;
    fn record_delivered_iou(&mut self);
    fn reload_check_after_iou_flow(&mut self);
    fn restore_iou_flow_limit(&mut self);
    fn remove_destination_dir(&mut self) -> bool;
    fn remove_owner_dir(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i32);
    fn erase_check(&mut self);
    fn apply_view(&mut self);
}

pub fn run_check_cash_preflight(facts: CheckCashPreflightFacts) -> NotTec {
    if facts.amount_present == facts.deliver_min_present {
        return Ter::TEM_MALFORMED;
    }

    if !facts.value_is_legal || !facts.value_signum_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.value_currency_is_bad {
        return Ter::TEM_BAD_CURRENCY;
    }

    Ter::TES_SUCCESS
}

pub fn run_check_cash_preclaim(facts: CheckCashPreclaimFacts) -> Ter {
    if !facts.check_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.tx_account_is_check_destination {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.check_source_is_destination {
        return Ter::TEC_INTERNAL;
    }

    if !facts.source_account_exists || !facts.destination_account_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if facts.destination_require_dest_tag && !facts.check_has_destination_tag {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    if facts.check_expired {
        return Ter::TEC_EXPIRED;
    }

    if !facts.requested_currency_matches_send_max || !facts.requested_issuer_matches_send_max {
        return Ter::TEM_MALFORMED;
    }

    if facts.requested_value_exceeds_send_max || facts.requested_value_exceeds_available_funds {
        return Ter::TEC_PATH_PARTIAL;
    }

    if !facts.requested_value_native && !facts.requested_value_issuer_is_destination {
        if !facts.issuer_exists {
            return Ter::TEC_NO_ISSUER;
        }

        if facts.issuer_requires_auth
            && (!facts.destination_trustline_exists || !facts.destination_trustline_authorized)
        {
            return Ter::TEC_NO_AUTH;
        }

        if facts.destination_trustline_frozen {
            return Ter::TEC_FROZEN;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_check_cash_select_requested_value<T>(
    amount: Option<T>,
    deliver_min: Option<T>,
) -> Option<T> {
    amount.or(deliver_min)
}

pub fn run_check_cash_compute_xrp_deliver<T>(
    requested_value: T,
    send_max: T,
    src_liquid: T,
    deliver_min_present: bool,
) -> T
where
    T: Ord,
{
    if deliver_min_present {
        std::cmp::max(requested_value, std::cmp::min(send_max, src_liquid))
    } else {
        requested_value
    }
}

pub fn run_check_cash_plan_iou_trustline<AccountId>(
    issuer: AccountId,
    source: AccountId,
    destination: AccountId,
    trustline_exists: bool,
    destination_can_pay_trustline_reserve: bool,
) -> Result<CheckCashIouTrustlinePlan<AccountId>, Ter>
where
    AccountId: Copy + Eq + Ord,
{
    if !trustline_exists && !destination_can_pay_trustline_reserve {
        return Err(Ter::TEC_NO_LINE_INSUF_RESERVE);
    }

    Ok(CheckCashIouTrustlinePlan {
        truster: if issuer == destination {
            source
        } else {
            destination
        },
        needs_create: !trustline_exists,
        tweaked_limit_side: if issuer > destination {
            CheckCashIouLimitSide::Low
        } else {
            CheckCashIouLimitSide::High
        },
    })
}

pub fn run_check_cash_plan_iou_trust_create<AccountId, Balance, Limit>(
    issuer: AccountId,
    destination: AccountId,
    trustline_exists: bool,
    destination_can_pay_trustline_reserve: bool,
    destination_default_ripple_enabled: bool,
    zero_initial_balance: Balance,
    zero_limit: Limit,
) -> Result<CheckCashIouTrustCreatePlan<Balance, Limit>, Ter>
where
    AccountId: Ord,
{
    if !trustline_exists && !destination_can_pay_trustline_reserve {
        return Err(Ter::TEC_NO_LINE_INSUF_RESERVE);
    }

    let field_shape = run_check_cash_plan_iou_trust_create_field_shape(issuer > destination);

    Ok(CheckCashIouTrustCreatePlan {
        needs_create: !trustline_exists,
        can_create: destination_can_pay_trustline_reserve,
        dest_low: field_shape.dest_low,
        authorize_account: false,
        no_ripple: !destination_default_ripple_enabled,
        freeze: false,
        deep_freeze: false,
        initial_balance: zero_initial_balance,
        limit: zero_limit,
        quality_in: 0,
        quality_out: 0,
    })
}

pub fn run_check_cash_plan_iou_trust_create_field_shape(
    dest_low: bool,
) -> CheckCashIouTrustCreateFieldShape {
    CheckCashIouTrustCreateFieldShape {
        dest_low,
        initial_balance_is_zero: true,
        destination_limit_is_zero: true,
    }
}

fn run_check_cash_plan_iou_trust_create_post_create_available_check(
    trustline_exists: bool,
) -> bool {
    !trustline_exists
}

pub fn run_check_cash_plan_iou_trust_create_destination_state(
    owner_count: u32,
    default_ripple_enabled: bool,
) -> CheckCashIouTrustCreateDestinationState {
    CheckCashIouTrustCreateDestinationState {
        owner_count,
        default_ripple_enabled,
    }
}

pub fn run_check_cash_plan_iou_trust_create_handoff<
    AccountId,
    TrustLineKey,
    TrustLineIndex,
    Balance,
    Limit,
>(
    issuer: AccountId,
    source: AccountId,
    destination: AccountId,
    trustline_key: TrustLineKey,
    trustline_index: TrustLineIndex,
    trustline_exists: bool,
    destination_can_pay_trustline_reserve: bool,
    destination_default_ripple_enabled: bool,
    zero_initial_balance: Balance,
    zero_limit: Limit,
) -> Result<
    CheckCashIouTrustCreateHandoffPlan<AccountId, TrustLineKey, TrustLineIndex, Balance, Limit>,
    Ter,
>
where
    AccountId: Copy + Eq + Ord,
{
    if !trustline_exists && !destination_can_pay_trustline_reserve {
        return Err(Ter::TEC_NO_LINE_INSUF_RESERVE);
    }

    Ok(CheckCashIouTrustCreateHandoffPlan {
        truster: if issuer == destination {
            source
        } else {
            destination
        },
        trustline_key,
        trustline_index,
        destination_update_after_create: !trustline_exists,
        needs_create: !trustline_exists,
        can_create: destination_can_pay_trustline_reserve,
        dest_low: issuer > destination,
        authorize_account: false,
        no_ripple: !destination_default_ripple_enabled,
        freeze: false,
        deep_freeze: false,
        initial_balance: zero_initial_balance,
        limit: zero_limit,
        quality_in: 0,
        quality_out: 0,
    })
}

pub fn run_check_cash_plan_iou_trust_create_caller<
    AccountId,
    TrustLineKey,
    TrustLineIndex,
    Balance,
    Limit,
>(
    issuer: AccountId,
    source: AccountId,
    destination: AccountId,
    trustline_key: TrustLineKey,
    trustline_index: TrustLineIndex,
    trustline_exists: bool,
    destination_can_pay_trustline_reserve: bool,
    destination_default_ripple_enabled: bool,
    destination_owner_count: u32,
    zero_initial_balance: Balance,
    zero_limit: Limit,
) -> Result<
    CheckCashIouTrustCreateCallerPlan<AccountId, TrustLineKey, TrustLineIndex, Balance, Limit>,
    Ter,
>
where
    AccountId: Copy + Eq + Ord,
{
    let destination_state = run_check_cash_plan_iou_trust_create_destination_state(
        destination_owner_count,
        destination_default_ripple_enabled,
    );
    let trust_create = run_check_cash_plan_iou_trust_create_handoff(
        issuer,
        source,
        destination,
        trustline_key,
        trustline_index,
        trustline_exists,
        destination_can_pay_trustline_reserve,
        destination_default_ripple_enabled,
        zero_initial_balance,
        zero_limit,
    )?;

    Ok(CheckCashIouTrustCreateCallerPlan {
        destination_state,
        trust_create,
    })
}

pub fn run_check_cash_build_iou_flow_deliver<T>(
    requested_value: T,
    deliver_min_present: bool,
    build_deliver_min_limit: impl FnOnce(T) -> T,
) -> T {
    if deliver_min_present {
        build_deliver_min_limit(requested_value)
    } else {
        requested_value
    }
}

pub fn run_check_cash_plan_iou_limit_override<AccountId, T>(
    issuer: AccountId,
    destination: AccountId,
    temporary_limit: T,
) -> CheckCashIouLimitOverridePlan<T>
where
    AccountId: Ord,
{
    CheckCashIouLimitOverridePlan {
        tweaked_limit_side: if issuer > destination {
            CheckCashIouLimitSide::Low
        } else {
            CheckCashIouLimitSide::High
        },
        temporary_limit,
    }
}

pub fn run_check_cash_plan_do_apply(
    source_is_destination: bool,
    send_max_is_native: bool,
) -> CheckCashDoApplyPlan {
    CheckCashDoApplyPlan {
        branch: if source_is_destination {
            CheckCashDoApplyBranch::SelfCash
        } else if send_max_is_native {
            CheckCashDoApplyBranch::Xrp
        } else {
            CheckCashDoApplyBranch::Iou
        },
        cleanup: CheckCashDoApplyCleanupPlan {
            remove_destination_dir: !source_is_destination,
            remove_owner_dir: true,
            adjust_owner_count_delta: -1,
            erase_check: true,
            apply_view: true,
        },
    }
}

pub fn run_check_cash_build_iou_flow_request<T>(
    requested_value: T,
    deliver_min_present: bool,
    build_deliver_min_limit: impl FnOnce(T) -> T,
) -> CheckCashIouFlowRequest<T> {
    CheckCashIouFlowRequest {
        deliver: run_check_cash_build_iou_flow_deliver(
            requested_value,
            deliver_min_present,
            build_deliver_min_limit,
        ),
        partial_payment: deliver_min_present,
        default_path: true,
        owner_pays_transfer_fee: true,
    }
}

pub fn run_check_cash_plan_iou_flow_call<AccountId, T>(
    issuer: AccountId,
    destination: AccountId,
    requested_value: T,
    deliver_min_present: bool,
    temporary_limit: T,
    build_deliver_min_limit: impl FnOnce(T) -> T,
) -> CheckCashIouFlowCallPlan<T>
where
    AccountId: Ord,
{
    CheckCashIouFlowCallPlan {
        request: run_check_cash_build_iou_flow_request(
            requested_value,
            deliver_min_present,
            build_deliver_min_limit,
        ),
        limit_override: run_check_cash_plan_iou_limit_override(
            issuer,
            destination,
            temporary_limit,
        ),
    }
}

pub fn run_check_cash_finish_iou_flow(
    flow: CheckCashIouFlowResult,
    amount_present: bool,
    deliver_min_present: bool,
) -> CheckCashIouFlowOutcome {
    if !is_tes_success(flow.ter) {
        return CheckCashIouFlowOutcome {
            ter: flow.ter,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        };
    }

    if amount_present && !flow.meets_requested_amount {
        return CheckCashIouFlowOutcome {
            ter: Ter::TEC_PATH_PARTIAL,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        };
    }

    if deliver_min_present && !flow.meets_deliver_min {
        return CheckCashIouFlowOutcome {
            ter: Ter::TEC_PATH_PARTIAL,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        };
    }

    CheckCashIouFlowOutcome {
        ter: Ter::TES_SUCCESS,
        record_delivered_amount: true,
        reload_check_after_flow: true,
    }
}

pub fn run_check_cash_do_apply_xrp_path<S: CheckCashApplySink>(
    deliver_min_present: bool,
    sink: &mut S,
) -> Ter {
    if !sink.xrp_liquid_sufficient() {
        return Ter::TEC_UNFUNDED_PAYMENT;
    }

    if deliver_min_present {
        sink.record_delivered_xrp();
    }

    sink.transfer_xrp()
}

pub fn run_check_cash_do_apply_iou_path<S: CheckCashApplySink>(
    trustline_exists: bool,
    destination_can_pay_trustline_reserve: bool,
    amount_present: bool,
    deliver_min_present: bool,
    sink: &mut S,
) -> Ter {
    let trust_create_plan = match run_check_cash_plan_iou_trust_create(
        true,
        false,
        trustline_exists,
        destination_can_pay_trustline_reserve,
        true,
        (),
        (),
    ) {
        Ok(plan) => plan,
        Err(ter) => return ter,
    };

    if trust_create_plan.needs_create {
        let trust_create = sink.create_iou_trustline();
        if !is_tes_success(trust_create) {
            return trust_create;
        }

        sink.update_destination_after_trustline_create();

        if run_check_cash_plan_iou_trust_create_post_create_available_check(trustline_exists)
            && !sink.trustline_available_after_create()
        {
            return Ter::TEC_NO_LINE;
        }
    }

    let prepare = sink.prepare_iou_flow_limit();
    if !is_tes_success(prepare) {
        return prepare;
    }

    let flow_result = sink.run_iou_flow(deliver_min_present);
    let flow_outcome =
        run_check_cash_finish_iou_flow(flow_result, amount_present, deliver_min_present);
    if !is_tes_success(flow_outcome.ter) {
        sink.restore_iou_flow_limit();
        return flow_outcome.ter;
    }

    if flow_outcome.record_delivered_amount {
        sink.record_delivered_iou();
    }
    if flow_outcome.reload_check_after_flow {
        sink.reload_check_after_iou_flow();
    }
    sink.restore_iou_flow_limit();
    flow_outcome.ter
}

pub fn run_check_cash_do_apply<S: CheckCashApplySink>(
    facts: CheckCashApplyFacts,
    sink: &mut S,
) -> Ter {
    if !facts.check_exists || !facts.source_account_exists || !facts.destination_account_exists {
        return Ter::TEC_FAILED_PROCESSING;
    }

    let plan = run_check_cash_plan_do_apply(facts.source_is_destination, facts.send_max_is_native);

    if !matches!(plan.branch, CheckCashDoApplyBranch::SelfCash) {
        let transfer = match plan.branch {
            CheckCashDoApplyBranch::Xrp => {
                run_check_cash_do_apply_xrp_path(facts.deliver_min_present, sink)
            }
            CheckCashDoApplyBranch::Iou => run_check_cash_do_apply_iou_path(
                facts.iou_trustline_exists,
                facts.iou_destination_can_pay_trustline_reserve,
                facts.amount_present,
                facts.deliver_min_present,
                sink,
            ),
            CheckCashDoApplyBranch::SelfCash => unreachable!("self-cash is handled above"),
        };

        if !is_tes_success(transfer) {
            return transfer;
        }
    }

    if plan.cleanup.remove_destination_dir && !sink.remove_destination_dir() {
        return Ter::TEF_BAD_LEDGER;
    }

    if plan.cleanup.remove_owner_dir && !sink.remove_owner_dir() {
        return Ter::TEF_BAD_LEDGER;
    }

    sink.adjust_owner_count(plan.cleanup.adjust_owner_count_delta);
    if plan.cleanup.erase_check {
        sink.erase_check();
    }
    if plan.cleanup.apply_view {
        sink.apply_view();
    }
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests;
