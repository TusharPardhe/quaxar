//! Rust compatibility surface for the reference implementation.
//!
//! This module preserves the current deterministic
//! `makeTxConsequences(...)`, `preflight(...)`, `preclaim(...)`, and `doApply()`
//! shell behavior around the surrounding keylet, ledger-read, directory, and
//! mutation work.

use crate::TxConsequences;
use crate::consequences::{TxConsequencesShape, build_tx_consequences};
use protocol::{NotTec, SeqProxy, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelCreatePreflightFacts {
    pub amount_is_xrp: bool,
    pub amount_positive: bool,
    pub tx_account_is_destination: bool,
    pub public_key_valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelCreatePreclaimFacts {
    pub source_account_exists: bool,
    pub source_balance_covers_reserve: bool,
    pub source_balance_covers_reserve_plus_amount: bool,
    pub destination_exists: bool,
    pub destination_disallow_incoming_pay_chan: bool,
    pub destination_requires_dest_tag: bool,
    pub destination_has_dest_tag: bool,
    pub destination_is_pseudo_account: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelCreateApplyFacts {
    pub account_exists: bool,
    pub fix_paychan_cancel_after_enabled: bool,
    pub cancel_after_expired: bool,
    pub include_sequence_field: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelCreatePreparedApplyFacts {
    pub account_exists: bool,
    pub cancel_after_invalid: bool,
    pub include_sequence_field: bool,
}

pub trait PaymentChannelCreateApplySink {
    fn create_payment_channel_entry(&mut self, include_sequence_field: bool);
    fn insert_owner_directory(&mut self) -> Option<u64>;
    fn set_owner_node(&mut self, page: u64);
    fn insert_destination_directory(&mut self) -> Option<u64>;
    fn set_destination_node(&mut self, page: u64);
    fn deduct_owner_balance(&mut self);
    fn adjust_owner_count(&mut self, delta: i32);
    fn update_owner_account(&mut self);
}

pub fn run_payment_channel_create_make_tx_consequences(
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

pub fn run_payment_channel_create_preflight(facts: PaymentChannelCreatePreflightFacts) -> NotTec {
    if !facts.amount_is_xrp || !facts.amount_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.tx_account_is_destination {
        return Ter::TEM_DST_IS_SRC;
    }

    if !facts.public_key_valid {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_payment_channel_create_preclaim(facts: PaymentChannelCreatePreclaimFacts) -> Ter {
    if !facts.source_account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.source_balance_covers_reserve {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    if !facts.source_balance_covers_reserve_plus_amount {
        return Ter::TEC_UNFUNDED;
    }

    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if facts.destination_disallow_incoming_pay_chan {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.destination_requires_dest_tag && !facts.destination_has_dest_tag {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    if facts.destination_is_pseudo_account {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn build_payment_channel_create_prepared_apply_facts(
    facts: PaymentChannelCreateApplyFacts,
) -> PaymentChannelCreatePreparedApplyFacts {
    PaymentChannelCreatePreparedApplyFacts {
        account_exists: facts.account_exists,
        cancel_after_invalid: facts.fix_paychan_cancel_after_enabled && facts.cancel_after_expired,
        include_sequence_field: facts.include_sequence_field,
    }
}

pub fn run_payment_channel_create_prepared_do_apply<S: PaymentChannelCreateApplySink>(
    prepared: PaymentChannelCreatePreparedApplyFacts,
    sink: &mut S,
) -> Ter {
    if !prepared.account_exists {
        return Ter::TEF_INTERNAL;
    }

    if prepared.cancel_after_invalid {
        return Ter::TEC_EXPIRED;
    }

    sink.create_payment_channel_entry(prepared.include_sequence_field);

    let owner_page = match sink.insert_owner_directory() {
        Some(page) => page,
        None => return Ter::TEC_DIR_FULL,
    };
    sink.set_owner_node(owner_page);

    let destination_page = match sink.insert_destination_directory() {
        Some(page) => page,
        None => return Ter::TEC_DIR_FULL,
    };
    sink.set_destination_node(destination_page);

    sink.deduct_owner_balance();
    sink.adjust_owner_count(1);
    sink.update_owner_account();
    Ter::TES_SUCCESS
}

pub fn run_payment_channel_create_do_apply<S: PaymentChannelCreateApplySink>(
    facts: PaymentChannelCreateApplyFacts,
    sink: &mut S,
) -> Ter {
    run_payment_channel_create_prepared_do_apply(
        build_payment_channel_create_prepared_apply_facts(facts),
        sink,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PaymentChannelCreateApplyFacts, PaymentChannelCreateApplySink,
        PaymentChannelCreatePreclaimFacts, PaymentChannelCreatePreflightFacts,
        PaymentChannelCreatePreparedApplyFacts, build_payment_channel_create_prepared_apply_facts,
        run_payment_channel_create_do_apply, run_payment_channel_create_make_tx_consequences,
        run_payment_channel_create_preclaim, run_payment_channel_create_preflight,
        run_payment_channel_create_prepared_do_apply,
    };
    use protocol::{SeqProxy, Ter};

    #[derive(Debug, Default)]
    struct TestApplySink {
        owner_dir_page: Option<u64>,
        destination_dir_page: Option<u64>,
        events: Vec<String>,
        owner_count_deltas: Vec<i32>,
        include_sequence_field: Option<bool>,
        owner_nodes: Vec<u64>,
        destination_nodes: Vec<u64>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                owner_dir_page: Some(11),
                destination_dir_page: Some(22),
                events: Vec::new(),
                owner_count_deltas: Vec::new(),
                include_sequence_field: None,
                owner_nodes: Vec::new(),
                destination_nodes: Vec::new(),
            }
        }
    }

    impl PaymentChannelCreateApplySink for TestApplySink {
        fn create_payment_channel_entry(&mut self, include_sequence_field: bool) {
            self.events.push("create".to_string());
            self.include_sequence_field = Some(include_sequence_field);
        }

        fn insert_owner_directory(&mut self) -> Option<u64> {
            self.events.push("owner_dir".to_string());
            self.owner_dir_page
        }

        fn set_owner_node(&mut self, page: u64) {
            self.events.push("set_owner_node".to_string());
            self.owner_nodes.push(page);
        }

        fn insert_destination_directory(&mut self) -> Option<u64> {
            self.events.push("destination_dir".to_string());
            self.destination_dir_page
        }

        fn set_destination_node(&mut self, page: u64) {
            self.events.push("set_destination_node".to_string());
            self.destination_nodes.push(page);
        }

        fn deduct_owner_balance(&mut self) {
            self.events.push("deduct_owner_balance".to_string());
        }

        fn adjust_owner_count(&mut self, delta: i32) {
            self.events.push(format!("adjust:{delta}"));
            self.owner_count_deltas.push(delta);
        }

        fn update_owner_account(&mut self) {
            self.events.push("update_owner".to_string());
        }
    }

    #[test]
    fn make_tx_consequences_tracks_cpp_potential_spend() {
        let consequences =
            run_payment_channel_create_make_tx_consequences(12, SeqProxy::sequence(9), 444);

        assert_eq!(consequences.fee(), 12);
        assert_eq!(consequences.seq_proxy(), SeqProxy::sequence(9));
        assert_eq!(consequences.potential_spend(), 444);
    }

    #[test]
    fn prepared_do_apply_matches_direct_do_apply() {
        let facts = PaymentChannelCreateApplyFacts {
            account_exists: true,
            fix_paychan_cancel_after_enabled: false,
            cancel_after_expired: false,
            include_sequence_field: true,
        };

        let prepared: PaymentChannelCreatePreparedApplyFacts =
            build_payment_channel_create_prepared_apply_facts(facts);

        let mut prepared_sink = TestApplySink::new();
        let prepared_result =
            run_payment_channel_create_prepared_do_apply(prepared, &mut prepared_sink);

        let mut direct_sink = TestApplySink::new();
        let direct_result = run_payment_channel_create_do_apply(facts, &mut direct_sink);

        assert_eq!(prepared_result, direct_result);
        assert_eq!(prepared_sink.events, direct_sink.events);
        assert_eq!(
            prepared_sink.include_sequence_field,
            direct_sink.include_sequence_field
        );
        assert_eq!(
            prepared_sink.owner_count_deltas,
            direct_sink.owner_count_deltas
        );
        assert_eq!(prepared_sink.owner_nodes, direct_sink.owner_nodes);
        assert_eq!(
            prepared_sink.destination_nodes,
            direct_sink.destination_nodes
        );
    }

    #[test]
    fn preflight_ordering() {
        assert_eq!(
            run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
                amount_is_xrp: false,
                amount_positive: true,
                tx_account_is_destination: false,
                public_key_valid: true,
            }),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
                amount_is_xrp: true,
                amount_positive: true,
                tx_account_is_destination: true,
                public_key_valid: true,
            }),
            Ter::TEM_DST_IS_SRC
        );
        assert_eq!(
            run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
                amount_is_xrp: true,
                amount_positive: true,
                tx_account_is_destination: false,
                public_key_valid: false,
            }),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn preclaim_ordering() {
        assert_eq!(
            run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
                source_account_exists: false,
                source_balance_covers_reserve: true,
                source_balance_covers_reserve_plus_amount: true,
                destination_exists: true,
                destination_disallow_incoming_pay_chan: false,
                destination_requires_dest_tag: false,
                destination_has_dest_tag: false,
                destination_is_pseudo_account: false,
            }),
            Ter::TER_NO_ACCOUNT
        );
        assert_eq!(
            run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
                source_account_exists: true,
                source_balance_covers_reserve: false,
                source_balance_covers_reserve_plus_amount: true,
                destination_exists: true,
                destination_disallow_incoming_pay_chan: false,
                destination_requires_dest_tag: false,
                destination_has_dest_tag: false,
                destination_is_pseudo_account: false,
            }),
            Ter::TEC_INSUFFICIENT_RESERVE
        );
        assert_eq!(
            run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
                source_account_exists: true,
                source_balance_covers_reserve: true,
                source_balance_covers_reserve_plus_amount: false,
                destination_exists: true,
                destination_disallow_incoming_pay_chan: false,
                destination_requires_dest_tag: false,
                destination_has_dest_tag: false,
                destination_is_pseudo_account: false,
            }),
            Ter::TEC_UNFUNDED
        );
        assert_eq!(
            run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
                source_account_exists: true,
                source_balance_covers_reserve: true,
                source_balance_covers_reserve_plus_amount: true,
                destination_exists: false,
                destination_disallow_incoming_pay_chan: false,
                destination_requires_dest_tag: false,
                destination_has_dest_tag: false,
                destination_is_pseudo_account: false,
            }),
            Ter::TEC_NO_DST
        );
    }

    #[test]
    fn do_apply_updates_directories_before_tail() {
        let mut sink = TestApplySink::new();

        let result = run_payment_channel_create_do_apply(
            PaymentChannelCreateApplyFacts {
                account_exists: true,
                fix_paychan_cancel_after_enabled: false,
                cancel_after_expired: false,
                include_sequence_field: true,
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "create",
                "owner_dir",
                "set_owner_node",
                "destination_dir",
                "set_destination_node",
                "deduct_owner_balance",
                "adjust:1",
                "update_owner",
            ]
        );
        assert_eq!(sink.include_sequence_field, Some(true));
        assert_eq!(sink.owner_count_deltas, vec![1]);
        assert_eq!(sink.owner_nodes, vec![11]);
        assert_eq!(sink.destination_nodes, vec![22]);
    }
}
