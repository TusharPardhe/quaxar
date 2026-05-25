//! Rust compatibility surface for the reference implementation.
//!
//! This module preserves the exact current deterministic
//! `makeTxConsequences(...)`, `preflight(...)`, and `doApply()` wrapper
//! behavior around the surrounding keylet, ledger-read, close-time, journal,
//! and mutation work.

use crate::TxConsequences;
use crate::consequences::{TxConsequencesShape, build_tx_consequences};
use crate::payment_channel_due::PaymentChannelDueFacts;
use crate::payment_channel_fund_loaded_destination_mutation_guarded_apply::run_payment_channel_fund_apply_destination_mutation_guarded_do_apply;
use crate::payment_channel_helpers::{PaymentChannelCloseFacts, PaymentChannelCloseSink};
use protocol::{NotTec, SeqProxy, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundApplyFacts<Time> {
    pub channel_exists: bool,
    pub due_facts: PaymentChannelDueFacts<Time>,
    pub close_facts: PaymentChannelCloseFacts,
    pub tx_account_is_owner: bool,
    pub channel_amount_drops: u64,
    pub fund_amount_drops: u64,
    pub extend_expiration: Option<Time>,
    pub min_extend_expiration: Time,
    pub owner_account_exists: bool,
    pub owner_balance_covers_reserve: bool,
    pub owner_balance_covers_reserve_plus_amount: bool,
    pub destination_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundPreparedDoApplyFacts<Time> {
    pub channel_exists: bool,
    pub due_facts: PaymentChannelDueFacts<Time>,
    pub close_facts: PaymentChannelCloseFacts,
    pub tx_account_is_owner: bool,
    pub channel_amount_drops: u64,
    pub fund_amount_drops: u64,
    pub extend_expiration: Option<Time>,
    pub min_extend_expiration: Time,
    pub owner_account_exists: bool,
    pub owner_balance_covers_reserve: bool,
    pub owner_balance_covers_reserve_plus_amount: bool,
    pub destination_exists: bool,
}

pub trait PaymentChannelFundApplySink<Time>: PaymentChannelCloseSink {
    fn update_expiration(&mut self, expiration: Time);
    fn set_channel_amount(&mut self, amount_drops: u64);
    fn persist_channel(&mut self);
    fn subtract_owner_balance(&mut self, amount_drops: u64);
    fn persist_owner(&mut self);
}

pub fn run_payment_channel_fund_make_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    amount_drops: u64,
) -> TxConsequences {
    build_tx_consequences(
        fee_drops,
        seq_proxy,
        TxConsequencesShape::PotentialSpend(amount_drops),
    )
}

pub fn run_payment_channel_fund_preflight(amount_is_xrp: bool, amount_positive: bool) -> NotTec {
    if !amount_is_xrp || !amount_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    Ter::TES_SUCCESS
}

pub fn run_payment_channel_fund_min_extend_expiration<Time>(
    parent_close_time: Time,
    settle_delay: Time,
    current_expiration: Option<Time>,
) -> Time
where
    Time: Copy + Ord + std::ops::Add<Output = Time>,
{
    let mut min_expiration = parent_close_time + settle_delay;
    if let Some(expiration) = current_expiration {
        if expiration < min_expiration {
            min_expiration = expiration;
        }
    }
    min_expiration
}

pub fn run_payment_channel_fund_do_apply<Time, S>(
    facts: PaymentChannelFundApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    run_payment_channel_fund_prepared_do_apply(
        build_payment_channel_fund_prepared_do_apply_facts(facts),
        sink,
    )
}

pub fn build_payment_channel_fund_prepared_do_apply_facts<Time>(
    facts: PaymentChannelFundApplyFacts<Time>,
) -> PaymentChannelFundPreparedDoApplyFacts<Time>
where
    Time: Copy,
{
    PaymentChannelFundPreparedDoApplyFacts {
        channel_exists: facts.channel_exists,
        due_facts: facts.due_facts,
        close_facts: facts.close_facts,
        tx_account_is_owner: facts.tx_account_is_owner,
        channel_amount_drops: facts.channel_amount_drops,
        fund_amount_drops: facts.fund_amount_drops,
        extend_expiration: facts.extend_expiration,
        min_extend_expiration: facts.min_extend_expiration,
        owner_account_exists: facts.owner_account_exists,
        owner_balance_covers_reserve: facts.owner_balance_covers_reserve,
        owner_balance_covers_reserve_plus_amount: facts.owner_balance_covers_reserve_plus_amount,
        destination_exists: facts.destination_exists,
    }
}

pub fn run_payment_channel_fund_prepared_do_apply<Time, S>(
    prepared: PaymentChannelFundPreparedDoApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    run_payment_channel_fund_apply_destination_mutation_guarded_do_apply(
        PaymentChannelFundApplyFacts {
            channel_exists: prepared.channel_exists,
            due_facts: prepared.due_facts,
            close_facts: prepared.close_facts,
            tx_account_is_owner: prepared.tx_account_is_owner,
            channel_amount_drops: prepared.channel_amount_drops,
            fund_amount_drops: prepared.fund_amount_drops,
            extend_expiration: prepared.extend_expiration,
            min_extend_expiration: prepared.min_extend_expiration,
            owner_account_exists: prepared.owner_account_exists,
            owner_balance_covers_reserve: prepared.owner_balance_covers_reserve,
            owner_balance_covers_reserve_plus_amount: prepared
                .owner_balance_covers_reserve_plus_amount,
            destination_exists: prepared.destination_exists,
        },
        sink,
    )
}

pub(crate) fn run_payment_channel_fund_core_apply<Time, S>(
    facts: PaymentChannelFundApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    if !facts.tx_account_is_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if let Some(expiration) = facts.extend_expiration {
        if expiration < facts.min_extend_expiration {
            return Ter::TEM_BAD_EXPIRATION;
        }
        sink.update_expiration(expiration);
    }

    if !facts.owner_account_exists {
        return Ter::TEF_INTERNAL;
    }

    if !facts.owner_balance_covers_reserve {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    if !facts.owner_balance_covers_reserve_plus_amount {
        return Ter::TEC_UNFUNDED;
    }

    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    let new_channel_amount = facts
        .channel_amount_drops
        .checked_add(facts.fund_amount_drops)
        .expect("input amount facts should stay within u64");
    sink.set_channel_amount(new_channel_amount);
    sink.persist_channel();
    sink.subtract_owner_balance(facts.fund_amount_drops);
    sink.persist_owner();
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{
        PaymentChannelFundApplyFacts, PaymentChannelFundApplySink,
        run_payment_channel_fund_do_apply, run_payment_channel_fund_make_tx_consequences,
        run_payment_channel_fund_min_extend_expiration, run_payment_channel_fund_preflight,
    };
    use crate::payment_channel_due::{PaymentChannelDueFacts, is_payment_channel_due};
    use crate::payment_channel_helpers::{
        PaymentChannelCloseFacts, PaymentChannelCloseSink, run_payment_channel_close,
    };
    use protocol::{SeqProxy, Ter};

    #[derive(Debug, Default)]
    struct TestApplySink {
        source_dir_result: Ter,
        destination_dir_result: Ter,
        source_account_exists: bool,
        events: Vec<String>,
        refund_drops: Option<u64>,
        owner_count_deltas: Vec<i32>,
        updated_expiration: Option<u32>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                source_dir_result: Ter::TES_SUCCESS,
                destination_dir_result: Ter::TES_SUCCESS,
                source_account_exists: true,
                events: Vec::new(),
                refund_drops: None,
                owner_count_deltas: Vec::new(),
                updated_expiration: None,
            }
        }
    }

    impl PaymentChannelCloseSink for TestApplySink {
        fn remove_source_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_source_owner_directory".to_string());
            self.source_dir_result
        }

        fn remove_destination_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_destination_owner_directory".to_string());
            self.destination_dir_result
        }

        fn source_account_exists(&mut self) -> bool {
            self.events.push("source_account_exists".to_string());
            self.source_account_exists
        }

        fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
            self.events
                .push("apply_refund_to_source_account".to_string());
            self.refund_drops = Some(refund_drops);
        }

        fn adjust_source_owner_count(&mut self, delta: i32) {
            self.events
                .push(format!("adjust_source_owner_count:{delta}"));
            self.owner_count_deltas.push(delta);
        }

        fn erase_channel(&mut self) {
            self.events.push("erase_channel".to_string());
        }
    }

    impl PaymentChannelFundApplySink<u32> for TestApplySink {
        fn update_expiration(&mut self, expiration: u32) {
            self.events.push("update_expiration".to_string());
            self.updated_expiration = Some(expiration);
        }

        fn set_channel_amount(&mut self, amount_drops: u64) {
            self.events
                .push(format!("set_channel_amount:{amount_drops}"));
        }

        fn persist_channel(&mut self) {
            self.events.push("persist_channel".to_string());
        }

        fn subtract_owner_balance(&mut self, amount_drops: u64) {
            self.events
                .push(format!("subtract_owner_balance:{amount_drops}"));
        }

        fn persist_owner(&mut self) {
            self.events.push("persist_owner".to_string());
        }
    }

    fn apply_facts() -> PaymentChannelFundApplyFacts<u32> {
        PaymentChannelFundApplyFacts {
            channel_exists: true,
            due_facts: PaymentChannelDueFacts {
                cancel_after: None,
                expiration: None,
                close_time: 0,
            },
            close_facts: PaymentChannelCloseFacts {
                destination_owner_directory_present: true,
                channel_amount_drops: 1_000,
                channel_balance_drops: 250,
            },
            tx_account_is_owner: true,
            channel_amount_drops: 1_000,
            fund_amount_drops: 300,
            extend_expiration: None,
            min_extend_expiration: 50,
            owner_account_exists: true,
            owner_balance_covers_reserve: true,
            owner_balance_covers_reserve_plus_amount: true,
            destination_exists: true,
        }
    }

    #[test]
    fn make_tx_consequences_tracks_cpp_potential_spend() {
        let consequences =
            run_payment_channel_fund_make_tx_consequences(12, SeqProxy::sequence(5), 300);

        assert_eq!(consequences.fee(), 12);
        assert_eq!(consequences.seq_proxy(), SeqProxy::sequence(5));
        assert_eq!(consequences.potential_spend(), 300);
    }

    #[test]
    fn preflight_rejects_bad_amount() {
        assert_eq!(
            run_payment_channel_fund_preflight(false, true),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_fund_preflight(true, false),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_fund_preflight(true, true),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn min_extend_expiration_rule() {
        assert_eq!(
            run_payment_channel_fund_min_extend_expiration(10_u32, 20, None),
            30
        );
        assert_eq!(
            run_payment_channel_fund_min_extend_expiration(10_u32, 20, Some(25)),
            25
        );
        assert_eq!(
            run_payment_channel_fund_min_extend_expiration(10_u32, 20, Some(45)),
            30
        );
    }

    #[test]
    fn due_helper_gate_rule() {
        assert!(is_payment_channel_due(PaymentChannelDueFacts {
            cancel_after: Some(10_u32),
            expiration: None,
            close_time: 10,
        }));
        assert!(is_payment_channel_due(PaymentChannelDueFacts {
            cancel_after: None,
            expiration: Some(10_u32),
            close_time: 11,
        }));
        assert!(!is_payment_channel_due(PaymentChannelDueFacts {
            cancel_after: Some(10_u32),
            expiration: Some(20),
            close_time: 9,
        }));
    }

    #[test]
    fn close_helper_returns_sink_result_unchanged() {
        let mut sink = TestApplySink::new();
        sink.source_dir_result = Ter::TEC_NO_DST;

        let result = run_payment_channel_close(
            PaymentChannelCloseFacts {
                destination_owner_directory_present: true,
                channel_amount_drops: 1_000,
                channel_balance_drops: 250,
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(sink.events, ["remove_source_owner_directory"]);
    }

    #[test]
    fn do_apply_due_channel_returns_close_helper_result_unchanged() {
        let close_facts = PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        };

        let mut helper_sink = TestApplySink::new();
        helper_sink.source_dir_result = Ter::TEC_NO_DST;
        let helper_result = run_payment_channel_close(close_facts, &mut helper_sink);

        let mut apply_sink = TestApplySink::new();
        apply_sink.source_dir_result = Ter::TEC_NO_DST;
        let mut facts = apply_facts();
        facts.due_facts = PaymentChannelDueFacts {
            cancel_after: None,
            expiration: Some(1),
            close_time: 1,
        };
        facts.tx_account_is_owner = false;
        facts.close_facts = close_facts;
        let apply_result = run_payment_channel_fund_do_apply(facts, &mut apply_sink);

        assert_eq!(apply_result, helper_result);
        assert_eq!(apply_sink.events, helper_sink.events);
    }

    #[test]
    fn do_apply_closes_due_channel_before_other_checks() {
        let mut sink = TestApplySink::new();
        sink.source_dir_result = Ter::TEF_BAD_LEDGER;

        let mut facts = apply_facts();
        facts.due_facts = PaymentChannelDueFacts {
            cancel_after: Some(1),
            expiration: None,
            close_time: 1,
        };
        facts.tx_account_is_owner = false;
        facts.close_facts = PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        };
        let result = run_payment_channel_fund_do_apply(facts, &mut sink);

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(sink.events, ["remove_source_owner_directory"]);
    }

    #[test]
    fn do_apply_updates_expiration_before_balance_checks() {
        let mut sink = TestApplySink::new();

        let result = run_payment_channel_fund_do_apply(
            PaymentChannelFundApplyFacts {
                extend_expiration: Some(55),
                owner_account_exists: false,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(sink.events, ["update_expiration"]);
        assert_eq!(sink.updated_expiration, Some(55));
    }
}
