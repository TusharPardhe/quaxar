//! the reference implementation compatibility surface.
//!
//! This ports the exact current deterministic behavior around:
//!
//! - custom `TxConsequences` sequence-consumption count,
//! - `preflight(...)` ticket-count bounds,
//! - `preclaim(...)` account-existence and ticket-threshold checks,
//! - and the full `doApply()` owner-load, reserve, sequence-sanity, ticket
//!   creation, owner-directory link, owner-count, ticket-count, and sequence
//!   update ordering.

use crate::TxConsequences;
use crate::consequences::{TxConsequencesShape, build_tx_consequences};
use protocol::{NotTec, SeqProxy, Ter};

pub const TICKET_CREATE_MIN_VALID_COUNT: u32 = 1;
pub const TICKET_CREATE_MAX_VALID_COUNT: u32 = 250;
pub const TICKET_CREATE_MAX_TICKET_THRESHOLD: u32 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TicketCreatePreclaimFacts {
    pub account_exists: bool,
    pub current_ticket_count: u32,
    pub requested_ticket_count: u32,
    pub consumes_ticket_sequence: bool,
}

pub trait TicketCreateDoApplySink {
    type OwnerNode: Copy;

    fn account_exists(&mut self) -> bool;
    fn has_reserve(&mut self, ticket_count: u32) -> bool;
    fn first_ticket_sequence(&mut self) -> u32;
    fn tx_sequence(&mut self) -> u32;
    fn create_ticket(&mut self, ticket_sequence: u32);
    fn dir_insert_ticket(&mut self, ticket_sequence: u32) -> Option<Self::OwnerNode>;
    fn set_ticket_owner_node(&mut self, ticket_sequence: u32, page: Self::OwnerNode);
    fn old_ticket_count(&mut self) -> u32;
    fn set_ticket_count(&mut self, ticket_count: u32);
    fn adjust_owner_count(&mut self, ticket_count: u32);
    fn set_account_sequence(&mut self, sequence: u32);
}

pub fn run_ticket_create_make_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    ticket_count: u32,
) -> TxConsequences {
    build_tx_consequences(
        fee_drops,
        seq_proxy,
        TxConsequencesShape::SequencesConsumed(ticket_count),
    )
}

pub fn run_ticket_create_preflight(ticket_count: u32) -> NotTec {
    if !(TICKET_CREATE_MIN_VALID_COUNT..=TICKET_CREATE_MAX_VALID_COUNT).contains(&ticket_count) {
        return Ter::TEM_INVALID_COUNT;
    }

    Ter::TES_SUCCESS
}

pub fn run_ticket_create_preclaim(facts: TicketCreatePreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    let consumed_tickets = facts.consumes_ticket_sequence as u32;
    if facts
        .current_ticket_count
        .wrapping_add(facts.requested_ticket_count)
        .wrapping_sub(consumed_tickets)
        > TICKET_CREATE_MAX_TICKET_THRESHOLD
    {
        return Ter::TEC_DIR_FULL;
    }

    Ter::TES_SUCCESS
}

pub fn run_ticket_create_do_apply<S: TicketCreateDoApplySink>(
    ticket_count: u32,
    sink: &mut S,
) -> Ter {
    if !sink.account_exists() {
        return Ter::TEF_INTERNAL;
    }

    if !sink.has_reserve(ticket_count) {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let first_ticket_sequence = sink.first_ticket_sequence();
    let tx_sequence = sink.tx_sequence();
    if tx_sequence != 0 && tx_sequence != first_ticket_sequence.wrapping_sub(1) {
        return Ter::TEF_INTERNAL;
    }

    for offset in 0..ticket_count {
        let ticket_sequence = first_ticket_sequence.wrapping_add(offset);
        sink.create_ticket(ticket_sequence);

        let Some(page) = sink.dir_insert_ticket(ticket_sequence) else {
            return Ter::TEC_DIR_FULL;
        };

        sink.set_ticket_owner_node(ticket_sequence, page);
    }

    let new_ticket_count = sink.old_ticket_count().wrapping_add(ticket_count);
    sink.set_ticket_count(new_ticket_count);
    sink.adjust_owner_count(ticket_count);
    sink.set_account_sequence(first_ticket_sequence.wrapping_add(ticket_count));

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::BTreeMap};

    use protocol::{SeqProxy, trans_token};

    use super::{
        TicketCreateDoApplySink, TicketCreatePreclaimFacts, run_ticket_create_do_apply,
        run_ticket_create_make_tx_consequences, run_ticket_create_preclaim,
        run_ticket_create_preflight,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestDoApplySink {
        account_exists: bool,
        has_reserve: bool,
        first_ticket_sequence: u32,
        tx_sequence: u32,
        old_ticket_count: u32,
        dir_pages: BTreeMap<u32, Option<u64>>,
        events: Vec<String>,
        final_ticket_count: Option<u32>,
        owner_count_delta: Option<u32>,
        final_account_sequence: Option<u32>,
        owner_nodes: Vec<(u32, u64)>,
    }

    impl TestDoApplySink {
        fn new() -> Self {
            let mut dir_pages = BTreeMap::new();
            dir_pages.insert(11, Some(101));
            dir_pages.insert(12, Some(102));

            Self {
                account_exists: true,
                has_reserve: true,
                first_ticket_sequence: 11,
                tx_sequence: 10,
                old_ticket_count: 3,
                dir_pages,
                events: Vec::new(),
                final_ticket_count: None,
                owner_count_delta: None,
                final_account_sequence: None,
                owner_nodes: Vec::new(),
            }
        }
    }

    impl TicketCreateDoApplySink for TestDoApplySink {
        type OwnerNode = u64;

        fn account_exists(&mut self) -> bool {
            self.events.push("account".to_string());
            self.account_exists
        }

        fn has_reserve(&mut self, ticket_count: u32) -> bool {
            self.events.push(format!("reserve:{ticket_count}"));
            self.has_reserve
        }

        fn first_ticket_sequence(&mut self) -> u32 {
            self.events.push("first_seq".to_string());
            self.first_ticket_sequence
        }

        fn tx_sequence(&mut self) -> u32 {
            self.events.push("tx_seq".to_string());
            self.tx_sequence
        }

        fn create_ticket(&mut self, ticket_sequence: u32) {
            self.events.push(format!("create:{ticket_sequence}"));
        }

        fn dir_insert_ticket(&mut self, ticket_sequence: u32) -> Option<Self::OwnerNode> {
            self.events.push(format!("dir:{ticket_sequence}"));
            self.dir_pages
                .get(&ticket_sequence)
                .copied()
                .unwrap_or(Some(1000 + u64::from(ticket_sequence)))
        }

        fn set_ticket_owner_node(&mut self, ticket_sequence: u32, page: Self::OwnerNode) {
            self.events
                .push(format!("owner_node:{ticket_sequence}:{page}"));
            self.owner_nodes.push((ticket_sequence, page));
        }

        fn old_ticket_count(&mut self) -> u32 {
            self.events.push("old_count".to_string());
            self.old_ticket_count
        }

        fn set_ticket_count(&mut self, ticket_count: u32) {
            self.events.push(format!("set_count:{ticket_count}"));
            self.final_ticket_count = Some(ticket_count);
        }

        fn adjust_owner_count(&mut self, ticket_count: u32) {
            self.events.push(format!("adjust:{ticket_count}"));
            self.owner_count_delta = Some(ticket_count);
        }

        fn set_account_sequence(&mut self, sequence: u32) {
            self.events.push(format!("set_seq:{sequence}"));
            self.final_account_sequence = Some(sequence);
        }
    }

    #[test]
    fn ticket_create_make_tx_consequences_consumes_requested_sequences() {
        let sequence = run_ticket_create_make_tx_consequences(12, SeqProxy::sequence(5), 3);
        let ticket = run_ticket_create_make_tx_consequences(12, SeqProxy::ticket(9), 2);

        assert_eq!(sequence.sequences_consumed(), 3);
        assert_eq!(sequence.following_seq(), SeqProxy::sequence(8));
        assert_eq!(ticket.sequences_consumed(), 2);
        assert_eq!(ticket.following_seq(), SeqProxy::ticket(11));
    }

    #[test]
    fn ticket_create_preflight_enforces_current_count_bounds() {
        assert_eq!(
            run_ticket_create_preflight(0),
            protocol::Ter::TEM_INVALID_COUNT
        );
        assert_eq!(
            run_ticket_create_preflight(251),
            protocol::Ter::TEM_INVALID_COUNT
        );
        assert_eq!(run_ticket_create_preflight(1), protocol::Ter::TES_SUCCESS);
        assert_eq!(run_ticket_create_preflight(250), protocol::Ter::TES_SUCCESS);
        assert_eq!(
            trans_token(run_ticket_create_preflight(0)),
            "temINVALID_COUNT"
        );
    }

    #[test]
    fn ticket_create_preclaim_rejects_missing_account() {
        let result = run_ticket_create_preclaim(TicketCreatePreclaimFacts {
            account_exists: false,
            current_ticket_count: 0,
            requested_ticket_count: 1,
            consumes_ticket_sequence: false,
        });

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }

    #[test]
    fn ticket_create_preclaim_preserves_current_ticket_threshold_math() {
        let at_limit_with_ticket = run_ticket_create_preclaim(TicketCreatePreclaimFacts {
            account_exists: true,
            current_ticket_count: 250,
            requested_ticket_count: 1,
            consumes_ticket_sequence: true,
        });
        let over_limit_with_sequence = run_ticket_create_preclaim(TicketCreatePreclaimFacts {
            account_exists: true,
            current_ticket_count: 250,
            requested_ticket_count: 1,
            consumes_ticket_sequence: false,
        });
        let over_limit_large_add = run_ticket_create_preclaim(TicketCreatePreclaimFacts {
            account_exists: true,
            current_ticket_count: 2,
            requested_ticket_count: 250,
            consumes_ticket_sequence: true,
        });

        assert_eq!(at_limit_with_ticket, protocol::Ter::TES_SUCCESS);
        assert_eq!(over_limit_with_sequence, protocol::Ter::TEC_DIR_FULL);
        assert_eq!(over_limit_large_add, protocol::Ter::TEC_DIR_FULL);
    }

    #[test]
    fn ticket_create_do_apply_returns_tefinternal_for_missing_owner() {
        let mut sink = TestDoApplySink::new();
        sink.account_exists = false;

        let result = run_ticket_create_do_apply(2, &mut sink);

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(sink.events, ["account"]);
    }

    #[test]
    fn ticket_create_do_apply_checks_reserve_before_sequence_work() {
        let mut sink = TestDoApplySink::new();
        sink.has_reserve = false;

        let result = run_ticket_create_do_apply(2, &mut sink);

        assert_eq!(result, protocol::Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(sink.events, ["account", "reserve:2"]);
    }

    #[test]
    fn ticket_create_do_apply_rejects_sequence_sanity_mismatch() {
        let mut sink = TestDoApplySink::new();
        sink.tx_sequence = 8;

        let result = run_ticket_create_do_apply(2, &mut sink);

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(sink.events, ["account", "reserve:2", "first_seq", "tx_seq"]);
    }

    #[test]
    fn ticket_create_do_apply_maps_dir_insert_failure() {
        let mut sink = TestDoApplySink::new();
        sink.dir_pages.insert(11, None);

        let result = run_ticket_create_do_apply(2, &mut sink);

        assert_eq!(result, protocol::Ter::TEC_DIR_FULL);
        assert_eq!(
            sink.events,
            [
                "account",
                "reserve:2",
                "first_seq",
                "tx_seq",
                "create:11",
                "dir:11"
            ]
        );
    }

    #[test]
    fn ticket_create_do_apply_preserves_current_on_success() {
        let mut sink = TestDoApplySink::new();
        sink.tx_sequence = 0;

        let result = run_ticket_create_do_apply(2, &mut sink);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "account",
                "reserve:2",
                "first_seq",
                "tx_seq",
                "create:11",
                "dir:11",
                "owner_node:11:101",
                "create:12",
                "dir:12",
                "owner_node:12:102",
                "old_count",
                "set_count:5",
                "adjust:2",
                "set_seq:13",
            ]
        );
        assert_eq!(sink.final_ticket_count, Some(5));
        assert_eq!(sink.owner_count_delta, Some(2));
        assert_eq!(sink.final_account_sequence, Some(13));
        assert_eq!(sink.owner_nodes, vec![(11, 101), (12, 102)]);
    }

    #[test]
    fn ticket_create_do_apply_wraps_ticket_count_and_sequence_updates() {
        let final_ticket_count = Cell::new(None);
        let final_sequence = Cell::new(None);

        struct WrappingSink<'a> {
            final_ticket_count: &'a Cell<Option<u32>>,
            final_sequence: &'a Cell<Option<u32>>,
        }

        impl TicketCreateDoApplySink for WrappingSink<'_> {
            type OwnerNode = u64;

            fn account_exists(&mut self) -> bool {
                true
            }

            fn has_reserve(&mut self, _: u32) -> bool {
                true
            }

            fn first_ticket_sequence(&mut self) -> u32 {
                u32::MAX
            }

            fn tx_sequence(&mut self) -> u32 {
                0
            }

            fn create_ticket(&mut self, _: u32) {}

            fn dir_insert_ticket(&mut self, _: u32) -> Option<Self::OwnerNode> {
                Some(1)
            }

            fn set_ticket_owner_node(&mut self, _: u32, _: Self::OwnerNode) {}

            fn old_ticket_count(&mut self) -> u32 {
                u32::MAX
            }

            fn set_ticket_count(&mut self, ticket_count: u32) {
                self.final_ticket_count.set(Some(ticket_count));
            }

            fn adjust_owner_count(&mut self, _: u32) {}

            fn set_account_sequence(&mut self, sequence: u32) {
                self.final_sequence.set(Some(sequence));
            }
        }

        let result = run_ticket_create_do_apply(
            2,
            &mut WrappingSink {
                final_ticket_count: &final_ticket_count,
                final_sequence: &final_sequence,
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(final_ticket_count.get(), Some(1));
        assert_eq!(final_sequence.get(), Some(1));
    }
}
