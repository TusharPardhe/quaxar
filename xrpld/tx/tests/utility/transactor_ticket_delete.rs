//! Integration tests that pin the narrowed Rust `Transactor::ticketDelete(...)`
//! helper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{TransactorTicketDeleteTicket, run_transactor_ticket_delete};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ticket {
    owner_page: u64,
    id: &'static str,
}

impl TransactorTicketDeleteTicket for Ticket {
    fn owner_page(&self) -> u64 {
        self.owner_page
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    ticket_count: Option<u32>,
    owner_count: i32,
}

#[test]
fn transactor_ticket_delete_maps_missing_ticket_and_account_shapes_to_bad_ledger() {
    let missing_ticket = run_transactor_ticket_delete(
        &"alice",
        &"ticket-1",
        None::<Ticket>,
        |_, _, _, _| panic!("missing ticket should skip dirRemove"),
        None::<&mut Account>,
        |_| panic!("missing ticket should skip ticket count"),
        |_, _| panic!("missing ticket should skip ticket-count writes"),
        |_, _| panic!("missing ticket should skip owner-count adjustment"),
        |_| panic!("missing ticket should skip erase"),
    );

    let account_missing = run_transactor_ticket_delete(
        &"alice",
        &"ticket-1",
        Some(Ticket {
            owner_page: 33,
            id: "ticket-1",
        }),
        |_, _, _, _| true,
        None::<&mut Account>,
        |_| panic!("missing account should skip ticket count"),
        |_, _| panic!("missing account should skip ticket-count writes"),
        |_, _| panic!("missing account should skip owner-count adjustment"),
        |_| panic!("missing account should skip erase"),
    );

    assert_eq!(missing_ticket, Ter::TEF_BAD_LEDGER);
    assert_eq!(account_missing, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(missing_ticket), "tefBAD_LEDGER");
}

#[test]
fn transactor_ticket_delete_maps_failed_dir_remove_or_missing_count_to_bad_ledger() {
    let mut dir_remove_account = Account {
        ticket_count: Some(3),
        owner_count: 9,
    };
    let dir_remove_failed = run_transactor_ticket_delete(
        &"alice",
        &"ticket-1",
        Some(Ticket {
            owner_page: 33,
            id: "ticket-1",
        }),
        |_, _, _, _| false,
        Some(&mut dir_remove_account),
        |_| panic!("failed dirRemove should skip ticket count"),
        |_, _| panic!("failed dirRemove should skip ticket-count writes"),
        |_, _| panic!("failed dirRemove should skip owner-count adjustment"),
        |_| panic!("failed dirRemove should skip erase"),
    );

    let mut missing_count_account = Account {
        ticket_count: None,
        owner_count: 9,
    };
    let missing_count = run_transactor_ticket_delete(
        &"alice",
        &"ticket-1",
        Some(Ticket {
            owner_page: 33,
            id: "ticket-1",
        }),
        |_, _, _, _| true,
        Some(&mut missing_count_account),
        |account| account.ticket_count,
        |_, _| panic!("missing count should skip ticket-count writes"),
        |_, _| panic!("missing count should skip owner-count adjustment"),
        |_| panic!("missing count should skip erase"),
    );

    assert_eq!(dir_remove_failed, Ter::TEF_BAD_LEDGER);
    assert_eq!(missing_count, Ter::TEF_BAD_LEDGER);
}

#[test]
fn transactor_ticket_delete_clears_one_ticket_count() {
    let mut account = Account {
        ticket_count: Some(1),
        owner_count: 9,
    };
    let mut erased = "";

    let result = run_transactor_ticket_delete(
        &"alice",
        &"ticket-1",
        Some(Ticket {
            owner_page: 44,
            id: "ticket-1",
        }),
        |account, page, ticket_index, keep_root| {
            assert_eq!(*account, "alice");
            assert_eq!(page, 44);
            assert_eq!(*ticket_index, "ticket-1");
            assert!(keep_root);
            true
        },
        Some(&mut account),
        |account| account.ticket_count,
        |account, next| account.ticket_count = next,
        |account, delta| account.owner_count += delta,
        |ticket| erased = ticket.id,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(account.ticket_count, None);
    assert_eq!(account.owner_count, 8);
    assert_eq!(erased, "ticket-1");
}

#[test]
fn transactor_ticket_delete_decrements_multi_ticket_count() {
    let mut account = Account {
        ticket_count: Some(4),
        owner_count: 9,
    };

    let result = run_transactor_ticket_delete(
        &"alice",
        &"ticket-2",
        Some(Ticket {
            owner_page: 55,
            id: "ticket-2",
        }),
        |_, _, _, _| true,
        Some(&mut account),
        |account| account.ticket_count,
        |account, next| account.ticket_count = next,
        |account, delta| account.owner_count += delta,
        |_| {},
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(account.ticket_count, Some(3));
    assert_eq!(account.owner_count, 8);
}
