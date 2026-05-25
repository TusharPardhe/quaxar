//! Integration tests that pin the narrowed Rust `Transactor::consumeSeqProxy`
//! shell to the current C++ behavior.

use protocol::{SeqProxy, Ter};
use tx::{
    TransactorTicketDeleteTicket, run_transactor_consume_seq_proxy, run_transactor_ticket_delete,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Account {
    sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ticket {
    owner_page: u64,
}

impl TransactorTicketDeleteTicket for Ticket {
    fn owner_page(&self) -> u64 {
        self.owner_page
    }
}

#[test]
fn tx_transactor_consume_seq_proxy_increments_sequence_numbers() {
    let mut account = Account { sequence: 7 };

    let result = run_transactor_consume_seq_proxy(
        SeqProxy::sequence(11),
        &mut account,
        |account, next_sequence| account.sequence = next_sequence,
        || panic!("sequence path should skip ticket deletion"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(account, Account { sequence: 12 });
}

#[test]
fn tx_transactor_consume_seq_proxy_delegates_ticket_path() {
    let mut account = Account { sequence: 7 };
    let mut ticket_owner = ();

    let result = run_transactor_consume_seq_proxy(
        SeqProxy::ticket(22),
        &mut account,
        |account, next_sequence| account.sequence = next_sequence,
        || {
            run_transactor_ticket_delete(
                &"alice",
                &"ticket-22",
                Some(Ticket { owner_page: 44 }),
                |account, page, ticket_index, keep_root| {
                    assert_eq!(*account, "alice");
                    assert_eq!(page, 44);
                    assert_eq!(*ticket_index, "ticket-22");
                    assert!(keep_root);
                    true
                },
                Some(&mut ticket_owner),
                |_| Some(1),
                |_, _| {},
                |_, _| {},
                |_| {},
            )
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(account, Account { sequence: 7 });
}
