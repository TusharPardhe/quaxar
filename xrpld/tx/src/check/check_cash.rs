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
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        CheckCashApplyFacts, CheckCashApplySink, CheckCashDoApplyBranch,
        CheckCashDoApplyCleanupPlan, CheckCashDoApplyPlan, CheckCashIouFlowCallPlan,
        CheckCashIouFlowOutcome, CheckCashIouFlowRequest, CheckCashIouFlowResult,
        CheckCashIouLimitOverridePlan, CheckCashIouLimitSide,
        CheckCashIouTrustCreateDestinationState, CheckCashIouTrustCreateFieldShape,
        CheckCashIouTrustCreateHandoffPlan, CheckCashIouTrustCreatePlan, CheckCashIouTrustlinePlan,
        CheckCashPreclaimFacts, CheckCashPreflightFacts, run_check_cash_build_iou_flow_deliver,
        run_check_cash_build_iou_flow_request, run_check_cash_compute_xrp_deliver,
        run_check_cash_do_apply, run_check_cash_do_apply_iou_path,
        run_check_cash_do_apply_xrp_path, run_check_cash_finish_iou_flow,
        run_check_cash_plan_do_apply, run_check_cash_plan_iou_flow_call,
        run_check_cash_plan_iou_limit_override, run_check_cash_plan_iou_trust_create,
        run_check_cash_plan_iou_trust_create_caller,
        run_check_cash_plan_iou_trust_create_destination_state,
        run_check_cash_plan_iou_trust_create_field_shape,
        run_check_cash_plan_iou_trust_create_handoff,
        run_check_cash_plan_iou_trust_create_post_create_available_check,
        run_check_cash_plan_iou_trustline, run_check_cash_preclaim, run_check_cash_preflight,
        run_check_cash_select_requested_value,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestApplySink {
        xrp_liquid_sufficient: bool,
        transfer_xrp_result: Ter,
        create_iou_trustline_result: Ter,
        trustline_available_after_create: bool,
        prepare_iou_flow_limit_result: Ter,
        run_iou_flow_result: CheckCashIouFlowResult,
        remove_destination_dir: bool,
        remove_owner_dir: bool,
        run_iou_flow_deliver_min_flags: Vec<bool>,
        owner_count_deltas: Vec<i32>,
        events: Vec<String>,
        erased: bool,
        applied: bool,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                xrp_liquid_sufficient: true,
                transfer_xrp_result: Ter::TES_SUCCESS,
                create_iou_trustline_result: Ter::TES_SUCCESS,
                trustline_available_after_create: true,
                prepare_iou_flow_limit_result: Ter::TES_SUCCESS,
                run_iou_flow_result: CheckCashIouFlowResult {
                    ter: Ter::TES_SUCCESS,
                    meets_requested_amount: true,
                    meets_deliver_min: true,
                },
                remove_destination_dir: true,
                remove_owner_dir: true,
                run_iou_flow_deliver_min_flags: Vec::new(),
                owner_count_deltas: Vec::new(),
                events: Vec::new(),
                erased: false,
                applied: false,
            }
        }
    }

    impl CheckCashApplySink for TestApplySink {
        fn xrp_liquid_sufficient(&mut self) -> bool {
            self.events.push("xrp_liquid".to_string());
            self.xrp_liquid_sufficient
        }

        fn record_delivered_xrp(&mut self) {
            self.events.push("deliver_xrp".to_string());
        }

        fn transfer_xrp(&mut self) -> Ter {
            self.events.push("transfer_xrp".to_string());
            self.transfer_xrp_result
        }

        fn create_iou_trustline(&mut self) -> Ter {
            self.events.push("create_iou_trustline".to_string());
            self.create_iou_trustline_result
        }

        fn update_destination_after_trustline_create(&mut self) {
            self.events
                .push("update_destination_after_trustline_create".to_string());
        }

        fn trustline_available_after_create(&mut self) -> bool {
            self.events
                .push("trustline_available_after_create".to_string());
            self.trustline_available_after_create
        }

        fn prepare_iou_flow_limit(&mut self) -> Ter {
            self.events.push("prepare_iou_flow_limit".to_string());
            self.prepare_iou_flow_limit_result
        }

        fn run_iou_flow(&mut self, deliver_min_present: bool) -> CheckCashIouFlowResult {
            self.events.push("run_iou_flow".to_string());
            self.run_iou_flow_deliver_min_flags
                .push(deliver_min_present);
            self.run_iou_flow_result
        }

        fn record_delivered_iou(&mut self) {
            self.events.push("deliver_iou".to_string());
        }

        fn reload_check_after_iou_flow(&mut self) {
            self.events.push("reload_check_after_iou_flow".to_string());
        }

        fn restore_iou_flow_limit(&mut self) {
            self.events.push("restore_iou_flow_limit".to_string());
        }

        fn remove_destination_dir(&mut self) -> bool {
            self.events.push("remove_destination".to_string());
            self.remove_destination_dir
        }

        fn remove_owner_dir(&mut self) -> bool {
            self.events.push("remove_owner".to_string());
            self.remove_owner_dir
        }

        fn adjust_owner_count(&mut self, delta: i32) {
            self.events.push(format!("adjust:{delta}"));
            self.owner_count_deltas.push(delta);
        }

        fn erase_check(&mut self) {
            self.events.push("erase".to_string());
            self.erased = true;
        }

        fn apply_view(&mut self) {
            self.events.push("apply".to_string());
            self.applied = true;
        }
    }

    fn preflight_facts() -> CheckCashPreflightFacts {
        CheckCashPreflightFacts {
            amount_present: true,
            deliver_min_present: false,
            value_is_legal: true,
            value_signum_positive: true,
            value_currency_is_bad: false,
        }
    }

    fn preclaim_facts() -> CheckCashPreclaimFacts {
        CheckCashPreclaimFacts {
            check_exists: true,
            tx_account_is_check_destination: true,
            check_source_is_destination: false,
            source_account_exists: true,
            destination_account_exists: true,
            destination_require_dest_tag: false,
            check_has_destination_tag: false,
            check_expired: false,
            requested_currency_matches_send_max: true,
            requested_issuer_matches_send_max: true,
            requested_value_exceeds_send_max: false,
            requested_value_exceeds_available_funds: false,
            requested_value_native: false,
            requested_value_issuer_is_destination: false,
            issuer_exists: true,
            issuer_requires_auth: false,
            destination_trustline_exists: true,
            destination_trustline_authorized: true,
            destination_trustline_frozen: false,
        }
    }

    fn apply_facts() -> CheckCashApplyFacts {
        CheckCashApplyFacts {
            check_exists: true,
            source_account_exists: true,
            destination_account_exists: true,
            source_is_destination: false,
            send_max_is_native: true,
            amount_present: true,
            deliver_min_present: false,
            iou_trustline_exists: true,
            iou_destination_can_pay_trustline_reserve: true,
        }
    }

    #[test]
    fn check_cash_preflight_requires_exactly_one_amount_field() {
        let missing = run_check_cash_preflight(CheckCashPreflightFacts {
            amount_present: false,
            deliver_min_present: false,
            ..preflight_facts()
        });
        let both = run_check_cash_preflight(CheckCashPreflightFacts {
            amount_present: true,
            deliver_min_present: true,
            ..preflight_facts()
        });

        assert_eq!(missing, Ter::TEM_MALFORMED);
        assert_eq!(both, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(missing), "temMALFORMED");
    }

    #[test]
    fn check_cash_preflight_rejects_bad_amount() {
        let result = run_check_cash_preflight(CheckCashPreflightFacts {
            value_is_legal: false,
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
        assert_eq!(trans_token(result), "temBAD_AMOUNT");
    }

    #[test]
    fn check_cash_preflight_rejects_bad_currency() {
        let result = run_check_cash_preflight(CheckCashPreflightFacts {
            value_currency_is_bad: true,
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_BAD_CURRENCY);
        assert_eq!(trans_token(result), "temBAD_CURRENCY");
    }

    #[test]
    fn check_cash_select_requested_value_prefers_amount_then_deliver_min() {
        assert_eq!(
            run_check_cash_select_requested_value(Some(5_u32), Some(7_u32)),
            Some(5)
        );
        assert_eq!(
            run_check_cash_select_requested_value::<u32>(None, Some(7)),
            Some(7)
        );
        assert_eq!(
            run_check_cash_select_requested_value::<u32>(None, None),
            None
        );
    }

    #[test]
    fn check_cash_preclaim_rejects_missing_check() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            check_exists: false,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
    }

    #[test]
    fn check_cash_preclaim_rejects_wrong_destination() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            tx_account_is_check_destination: false,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
        assert_eq!(trans_token(result), "tecNO_PERMISSION");
    }

    #[test]
    fn check_cash_preclaim_maps_self_check_to_internal() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            check_source_is_destination: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
    }

    #[test]
    fn check_cash_preclaim_rejects_missing_source_or_destination() {
        let missing_source = run_check_cash_preclaim(CheckCashPreclaimFacts {
            source_account_exists: false,
            ..preclaim_facts()
        });
        let missing_destination = run_check_cash_preclaim(CheckCashPreclaimFacts {
            destination_account_exists: false,
            ..preclaim_facts()
        });

        assert_eq!(missing_source, Ter::TEC_NO_ENTRY);
        assert_eq!(missing_destination, Ter::TEC_NO_ENTRY);
    }

    #[test]
    fn check_cash_preclaim_rejects_required_destination_tag() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            destination_require_dest_tag: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
        assert_eq!(trans_token(result), "tecDST_TAG_NEEDED");
    }

    #[test]
    fn check_cash_preclaim_rejects_expired_check() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            check_expired: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_EXPIRED);
        assert_eq!(trans_token(result), "tecEXPIRED");
    }

    #[test]
    fn check_cash_preclaim_rejects_currency_or_issuer_mismatch() {
        let currency = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_currency_matches_send_max: false,
            ..preclaim_facts()
        });
        let issuer = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_issuer_matches_send_max: false,
            ..preclaim_facts()
        });

        assert_eq!(currency, Ter::TEM_MALFORMED);
        assert_eq!(issuer, Ter::TEM_MALFORMED);
    }

    #[test]
    fn check_cash_preclaim_rejects_value_above_sendmax_or_funds() {
        let send_max = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_value_exceeds_send_max: true,
            ..preclaim_facts()
        });
        let funds = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_value_exceeds_available_funds: true,
            ..preclaim_facts()
        });

        assert_eq!(send_max, Ter::TEC_PATH_PARTIAL);
        assert_eq!(funds, Ter::TEC_PATH_PARTIAL);
        assert_eq!(trans_token(send_max), "tecPATH_PARTIAL");
    }

    #[test]
    fn check_cash_preclaim_rejects_missing_or_unauthorized_issuer_line() {
        let no_issuer = run_check_cash_preclaim(CheckCashPreclaimFacts {
            issuer_exists: false,
            ..preclaim_facts()
        });
        let no_line = run_check_cash_preclaim(CheckCashPreclaimFacts {
            issuer_requires_auth: true,
            destination_trustline_exists: false,
            ..preclaim_facts()
        });
        let unauthorized = run_check_cash_preclaim(CheckCashPreclaimFacts {
            issuer_requires_auth: true,
            destination_trustline_authorized: false,
            ..preclaim_facts()
        });

        assert_eq!(no_issuer, Ter::TEC_NO_ISSUER);
        assert_eq!(no_line, Ter::TEC_NO_AUTH);
        assert_eq!(unauthorized, Ter::TEC_NO_AUTH);
    }

    #[test]
    fn check_cash_preclaim_rejects_frozen_destination_line() {
        let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
            destination_trustline_frozen: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_FROZEN);
        assert_eq!(trans_token(result), "tecFROZEN");
    }

    #[test]
    fn check_cash_compute_xrp_deliver_clamps_with_deliver_min_formula() {
        assert_eq!(
            run_check_cash_compute_xrp_deliver(80_u64, 120, 90, false),
            80
        );
        assert_eq!(
            run_check_cash_compute_xrp_deliver(80_u64, 120, 90, true),
            90
        );
        assert_eq!(run_check_cash_compute_xrp_deliver(80_u64, 70, 60, true), 80);
    }

    #[test]
    fn check_cash_plan_iou_trustline_maps_cpp_truster_and_limit_side() {
        let create_plan = run_check_cash_plan_iou_trustline(30_u32, 10, 20, false, true).unwrap();
        let issuer_is_destination =
            run_check_cash_plan_iou_trustline(20_u32, 10, 20, true, true).unwrap();

        assert_eq!(
            create_plan,
            CheckCashIouTrustlinePlan {
                truster: 20,
                needs_create: true,
                tweaked_limit_side: CheckCashIouLimitSide::Low,
            }
        );
        assert_eq!(
            issuer_is_destination,
            CheckCashIouTrustlinePlan {
                truster: 10,
                needs_create: false,
                tweaked_limit_side: CheckCashIouLimitSide::High,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trustline_rejects_missing_reserve() {
        let result = run_check_cash_plan_iou_trustline(30_u32, 10, 20, false, false);

        assert_eq!(result, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
    }

    #[test]
    fn check_cash_plan_iou_trust_create_reports_cpp_reserve_gate() {
        let create =
            run_check_cash_plan_iou_trust_create(30_u32, 20_u32, false, true, false, -1_i32, 0_i32)
                .unwrap();
        let existing =
            run_check_cash_plan_iou_trust_create(20_u32, 20_u32, true, false, true, -1_i32, 0_i32)
                .unwrap();
        let missing_reserve = run_check_cash_plan_iou_trust_create(
            30_u32, 20_u32, false, false, false, -1_i32, 0_i32,
        );

        assert_eq!(
            create,
            CheckCashIouTrustCreatePlan {
                needs_create: true,
                can_create: true,
                dest_low: true,
                authorize_account: false,
                no_ripple: true,
                freeze: false,
                deep_freeze: false,
                initial_balance: -1,
                limit: 0,
                quality_in: 0,
                quality_out: 0,
            }
        );
        assert_eq!(
            existing,
            CheckCashIouTrustCreatePlan {
                needs_create: false,
                can_create: false,
                dest_low: false,
                authorize_account: false,
                no_ripple: false,
                freeze: false,
                deep_freeze: false,
                initial_balance: -1,
                limit: 0,
                quality_in: 0,
                quality_out: 0,
            }
        );
        assert_eq!(missing_reserve, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
    }

    #[test]
    fn check_cash_plan_iou_trust_create_handoff_selects_truster_and_carries_identity() {
        let plan = run_check_cash_plan_iou_trust_create_handoff(
            30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, true, false, -1_i32, 0_i32,
        )
        .unwrap();

        assert_eq!(
            plan,
            CheckCashIouTrustCreateHandoffPlan {
                truster: 20,
                trustline_key: 77,
                trustline_index: 88,
                destination_update_after_create: true,
                needs_create: true,
                can_create: true,
                dest_low: true,
                authorize_account: false,
                no_ripple: true,
                freeze: false,
                deep_freeze: false,
                initial_balance: -1,
                limit: 0,
                quality_in: 0,
                quality_out: 0,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trust_create_handoff_uses_source_when_issuer_matches_destination() {
        let plan = run_check_cash_plan_iou_trust_create_handoff(
            20_u32, 10_u32, 20_u32, 99_u32, 100_u32, true, false, true, -1_i32, 0_i32,
        )
        .unwrap();

        assert_eq!(
            plan,
            CheckCashIouTrustCreateHandoffPlan {
                truster: 10,
                trustline_key: 99,
                trustline_index: 100,
                destination_update_after_create: false,
                needs_create: false,
                can_create: false,
                dest_low: false,
                authorize_account: false,
                no_ripple: false,
                freeze: false,
                deep_freeze: false,
                initial_balance: -1,
                limit: 0,
                quality_in: 0,
                quality_out: 0,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trust_create_handoff_rejects_missing_reserve() {
        let result = run_check_cash_plan_iou_trust_create_handoff(
            30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, false, false, -1_i32, 0_i32,
        );

        assert_eq!(result, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
    }

    #[test]
    fn check_cash_plan_iou_trust_create_caller_packages_destination_state_and_callback() {
        let plan = run_check_cash_plan_iou_trust_create_caller(
            30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, true, false, 4, -1_i32, 0_i32,
        )
        .unwrap();

        assert_eq!(
            plan.destination_state,
            CheckCashIouTrustCreateDestinationState {
                owner_count: 4,
                default_ripple_enabled: false,
            }
        );
        assert_eq!(
            plan.trust_create,
            CheckCashIouTrustCreateHandoffPlan {
                truster: 20,
                trustline_key: 77,
                trustline_index: 88,
                destination_update_after_create: true,
                needs_create: true,
                can_create: true,
                dest_low: true,
                authorize_account: false,
                no_ripple: true,
                freeze: false,
                deep_freeze: false,
                initial_balance: -1,
                limit: 0,
                quality_in: 0,
                quality_out: 0,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trust_create_field_shape() {
        let low = run_check_cash_plan_iou_trust_create_field_shape(true);
        let high = run_check_cash_plan_iou_trust_create_field_shape(false);

        assert_eq!(
            low,
            CheckCashIouTrustCreateFieldShape {
                dest_low: true,
                initial_balance_is_zero: true,
                destination_limit_is_zero: true,
            }
        );
        assert_eq!(
            high,
            CheckCashIouTrustCreateFieldShape {
                dest_low: false,
                initial_balance_is_zero: true,
                destination_limit_is_zero: true,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trust_create_destination_state_keeps_account_snapshot() {
        assert_eq!(
            run_check_cash_plan_iou_trust_create_destination_state(9, true),
            CheckCashIouTrustCreateDestinationState {
                owner_count: 9,
                default_ripple_enabled: true,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_trust_create_post_create_check() {
        assert!(run_check_cash_plan_iou_trust_create_post_create_available_check(false));
        assert!(!run_check_cash_plan_iou_trust_create_post_create_available_check(true));
    }

    #[test]
    fn check_cash_plan_do_apply_selects_branch_and_cleanup() {
        let self_cash = run_check_cash_plan_do_apply(true, true);
        let xrp = run_check_cash_plan_do_apply(false, true);
        let iou = run_check_cash_plan_do_apply(false, false);

        assert_eq!(
            self_cash,
            CheckCashDoApplyPlan {
                branch: CheckCashDoApplyBranch::SelfCash,
                cleanup: CheckCashDoApplyCleanupPlan {
                    remove_destination_dir: false,
                    remove_owner_dir: true,
                    adjust_owner_count_delta: -1,
                    erase_check: true,
                    apply_view: true,
                },
            }
        );
        assert_eq!(
            xrp,
            CheckCashDoApplyPlan {
                branch: CheckCashDoApplyBranch::Xrp,
                cleanup: CheckCashDoApplyCleanupPlan {
                    remove_destination_dir: true,
                    remove_owner_dir: true,
                    adjust_owner_count_delta: -1,
                    erase_check: true,
                    apply_view: true,
                },
            }
        );
        assert_eq!(
            iou,
            CheckCashDoApplyPlan {
                branch: CheckCashDoApplyBranch::Iou,
                cleanup: CheckCashDoApplyCleanupPlan {
                    remove_destination_dir: true,
                    remove_owner_dir: true,
                    adjust_owner_count_delta: -1,
                    erase_check: true,
                    apply_view: true,
                },
            }
        );
    }

    #[test]
    fn check_cash_build_iou_flow_deliver_uses_deliver_min_limit_only_when_needed() {
        let mut invoked = false;
        let direct = run_check_cash_build_iou_flow_deliver(55_u64, false, |_| {
            invoked = true;
            999
        });
        assert_eq!(direct, 55);
        assert!(!invoked);

        let limited = run_check_cash_build_iou_flow_deliver(55_u64, true, |value| value + 500);
        assert_eq!(limited, 555);
    }

    #[test]
    fn check_cash_plan_iou_limit_override_uses_cpp_limit_side_rule() {
        let low = run_check_cash_plan_iou_limit_override(30_u32, 20_u32, 999_u64);
        let high = run_check_cash_plan_iou_limit_override(20_u32, 20_u32, 999_u64);

        assert_eq!(
            low,
            CheckCashIouLimitOverridePlan {
                tweaked_limit_side: CheckCashIouLimitSide::Low,
                temporary_limit: 999,
            }
        );
        assert_eq!(
            high,
            CheckCashIouLimitOverridePlan {
                tweaked_limit_side: CheckCashIouLimitSide::High,
                temporary_limit: 999,
            }
        );
    }

    #[test]
    fn check_cash_build_iou_flow_request_flags() {
        let request = run_check_cash_build_iou_flow_request(55_u64, true, |value| value + 500);

        assert_eq!(
            request,
            CheckCashIouFlowRequest {
                deliver: 555,
                partial_payment: true,
                default_path: true,
                owner_pays_transfer_fee: true,
            }
        );
    }

    #[test]
    fn check_cash_plan_iou_flow_call_packages_request_and_limit_override() {
        let plan =
            run_check_cash_plan_iou_flow_call(30_u32, 20_u32, 55_u64, true, 999_u64, |value| {
                value + 500
            });

        assert_eq!(
            plan,
            CheckCashIouFlowCallPlan {
                request: CheckCashIouFlowRequest {
                    deliver: 555,
                    partial_payment: true,
                    default_path: true,
                    owner_pays_transfer_fee: true,
                },
                limit_override: CheckCashIouLimitOverridePlan {
                    tweaked_limit_side: CheckCashIouLimitSide::Low,
                    temporary_limit: 999,
                },
            }
        );
    }

    #[test]
    fn check_cash_finish_iou_flow_maps_cpp_result_rules() {
        let failure = run_check_cash_finish_iou_flow(
            CheckCashIouFlowResult {
                ter: Ter::TER_NO_RIPPLE,
                meets_requested_amount: true,
                meets_deliver_min: false,
            },
            false,
            true,
        );
        let partial = run_check_cash_finish_iou_flow(
            CheckCashIouFlowResult {
                ter: Ter::TES_SUCCESS,
                meets_requested_amount: true,
                meets_deliver_min: false,
            },
            false,
            true,
        );
        let exact_amount_partial = run_check_cash_finish_iou_flow(
            CheckCashIouFlowResult {
                ter: Ter::TES_SUCCESS,
                meets_requested_amount: false,
                meets_deliver_min: true,
            },
            true,
            false,
        );
        let success = run_check_cash_finish_iou_flow(
            CheckCashIouFlowResult {
                ter: Ter::TES_SUCCESS,
                meets_requested_amount: true,
                meets_deliver_min: true,
            },
            false,
            false,
        );

        assert_eq!(
            failure,
            CheckCashIouFlowOutcome {
                ter: Ter::TER_NO_RIPPLE,
                record_delivered_amount: false,
                reload_check_after_flow: false,
            }
        );
        assert_eq!(
            partial,
            CheckCashIouFlowOutcome {
                ter: Ter::TEC_PATH_PARTIAL,
                record_delivered_amount: false,
                reload_check_after_flow: false,
            }
        );
        assert_eq!(
            exact_amount_partial,
            CheckCashIouFlowOutcome {
                ter: Ter::TEC_PATH_PARTIAL,
                record_delivered_amount: false,
                reload_check_after_flow: false,
            }
        );
        assert_eq!(
            success,
            CheckCashIouFlowOutcome {
                ter: Ter::TES_SUCCESS,
                record_delivered_amount: true,
                reload_check_after_flow: true,
            }
        );
    }

    #[test]
    fn check_cash_preclaim_skips_issuer_checks_for_native_or_issuer_destination() {
        let native = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_value_native: true,
            issuer_exists: false,
            ..preclaim_facts()
        });
        let issuer_is_destination = run_check_cash_preclaim(CheckCashPreclaimFacts {
            requested_value_issuer_is_destination: true,
            issuer_exists: false,
            ..preclaim_facts()
        });

        assert_eq!(native, Ter::TES_SUCCESS);
        assert_eq!(issuer_is_destination, Ter::TES_SUCCESS);
    }

    #[test]
    fn check_cash_do_apply_xrp_path_maps_unfunded_payment() {
        let mut sink = TestApplySink::new();
        sink.xrp_liquid_sufficient = false;

        let result = run_check_cash_do_apply_xrp_path(false, &mut sink);

        assert_eq!(result, Ter::TEC_UNFUNDED_PAYMENT);
        assert_eq!(trans_token(result), "tecUNFUNDED_PAYMENT");
        assert_eq!(sink.events, ["xrp_liquid"]);
    }

    #[test]
    fn check_cash_do_apply_xrp_path_records_deliver_before_transfer() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply_xrp_path(true, &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
    }

    #[test]
    fn check_cash_do_apply_xrp_path_skips_deliver_for_amount_branch() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply_xrp_path(false, &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["xrp_liquid", "transfer_xrp"]);
    }

    #[test]
    fn check_cash_do_apply_xrp_path_returns_transfer_failure_unchanged() {
        let mut sink = TestApplySink::new();
        sink.transfer_xrp_result = Ter::TEC_INSUFFICIENT_RESERVE;

        let result = run_check_cash_do_apply_xrp_path(true, &mut sink);

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
    }

    #[test]
    fn check_cash_do_apply_iou_path_rejects_insufficient_reserve() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply_iou_path(false, false, true, false, &mut sink);

        assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
        assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
        assert!(sink.events.is_empty());
    }

    #[test]
    fn check_cash_do_apply_iou_path_returns_trust_create_failure_unchanged() {
        let mut sink = TestApplySink::new();
        sink.create_iou_trustline_result = Ter::TER_NO_RIPPLE;

        let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(sink.events, ["create_iou_trustline"]);
    }

    #[test]
    fn check_cash_do_apply_iou_path_returns_prepare_failure_unchanged() {
        let mut sink = TestApplySink::new();
        sink.prepare_iou_flow_limit_result = Ter::TEC_NO_LINE;

        let result = run_check_cash_do_apply_iou_path(true, true, true, false, &mut sink);

        assert_eq!(result, Ter::TEC_NO_LINE);
        assert_eq!(trans_token(result), "tecNO_LINE");
        assert_eq!(sink.events, ["prepare_iou_flow_limit"]);
    }

    #[test]
    fn check_cash_do_apply_iou_path_maps_missing_trustline_after_create() {
        let mut sink = TestApplySink::new();
        sink.trustline_available_after_create = false;

        let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

        assert_eq!(result, Ter::TEC_NO_LINE);
        assert_eq!(
            sink.events,
            [
                "create_iou_trustline",
                "update_destination_after_trustline_create",
                "trustline_available_after_create"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_iou_path_restores_limit_on_flow_failure() {
        let mut sink = TestApplySink::new();
        sink.run_iou_flow_result = CheckCashIouFlowResult {
            ter: Ter::TER_NO_RIPPLE,
            meets_requested_amount: true,
            meets_deliver_min: false,
        };

        let result = run_check_cash_do_apply_iou_path(true, true, false, true, &mut sink);

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(
            sink.events,
            [
                "prepare_iou_flow_limit",
                "run_iou_flow",
                "restore_iou_flow_limit"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_iou_path_maps_unsatisfied_deliver_min() {
        let mut sink = TestApplySink::new();
        sink.run_iou_flow_result = CheckCashIouFlowResult {
            ter: Ter::TES_SUCCESS,
            meets_requested_amount: true,
            meets_deliver_min: false,
        };

        let result = run_check_cash_do_apply_iou_path(true, true, false, true, &mut sink);

        assert_eq!(result, Ter::TEC_PATH_PARTIAL);
        assert_eq!(
            sink.events,
            [
                "prepare_iou_flow_limit",
                "run_iou_flow",
                "restore_iou_flow_limit"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_iou_path_runs_success_path_in() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply_iou_path(false, true, false, true, &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![true]);
        assert_eq!(
            sink.events,
            [
                "create_iou_trustline",
                "update_destination_after_trustline_create",
                "trustline_available_after_create",
                "prepare_iou_flow_limit",
                "run_iou_flow",
                "deliver_iou",
                "reload_check_after_iou_flow",
                "restore_iou_flow_limit"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_iou_path_records_delivery_and_reload_without_deliver_min() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![false]);
        assert_eq!(
            sink.events,
            [
                "create_iou_trustline",
                "update_destination_after_trustline_create",
                "trustline_available_after_create",
                "prepare_iou_flow_limit",
                "run_iou_flow",
                "deliver_iou",
                "reload_check_after_iou_flow",
                "restore_iou_flow_limit"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_maps_missing_check_to_failed_processing() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                check_exists: false,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_FAILED_PROCESSING);
        assert_eq!(trans_token(result), "tecFAILED_PROCESSING");
        assert!(sink.events.is_empty());
    }

    #[test]
    fn check_cash_do_apply_maps_missing_accounts_to_failed_processing() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                source_account_exists: false,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_FAILED_PROCESSING);
        assert!(sink.events.is_empty());
    }

    #[test]
    fn check_cash_do_apply_returns_xrp_failure_before_cleanup() {
        let mut sink = TestApplySink::new();
        sink.transfer_xrp_result = Ter::TEC_INSUFFICIENT_RESERVE;

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                deliver_min_present: true,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
    }

    #[test]
    fn check_cash_do_apply_delegates_iou_branch_with_deliver_min_flag() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                send_max_is_native: false,
                deliver_min_present: true,
                iou_trustline_exists: false,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![true]);
        assert_eq!(
            sink.events,
            [
                "create_iou_trustline",
                "update_destination_after_trustline_create",
                "trustline_available_after_create",
                "prepare_iou_flow_limit",
                "run_iou_flow",
                "deliver_iou",
                "reload_check_after_iou_flow",
                "restore_iou_flow_limit",
                "remove_destination",
                "remove_owner",
                "adjust:-1",
                "erase",
                "apply"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_skips_transfer_and_destination_remove_for_self_cash() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                source_is_destination: true,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["remove_owner", "adjust:-1", "erase", "apply"]);
    }

    #[test]
    fn check_cash_do_apply_maps_destination_remove_failure() {
        let mut sink = TestApplySink::new();
        sink.remove_destination_dir = false;

        let result = run_check_cash_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(
            sink.events,
            ["xrp_liquid", "transfer_xrp", "remove_destination"]
        );
    }

    #[test]
    fn check_cash_do_apply_maps_owner_remove_failure() {
        let mut sink = TestApplySink::new();
        sink.remove_owner_dir = false;

        let result = run_check_cash_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(
            sink.events,
            [
                "xrp_liquid",
                "transfer_xrp",
                "remove_destination",
                "remove_owner"
            ]
        );
    }

    #[test]
    fn check_cash_do_apply_runs_success_path_in() {
        let mut sink = TestApplySink::new();

        let result = run_check_cash_do_apply(
            CheckCashApplyFacts {
                deliver_min_present: true,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "xrp_liquid",
                "deliver_xrp",
                "transfer_xrp",
                "remove_destination",
                "remove_owner",
                "adjust:-1",
                "erase",
                "apply"
            ]
        );
        assert_eq!(sink.owner_count_deltas, vec![-1]);
        assert!(sink.erased);
        assert!(sink.applied);
    }
}
