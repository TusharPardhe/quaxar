//! Current Rust helper mirroring `Transactor::ticketDelete(...)`.
//!
//! This module preserves the exact current ordered behavior around:
//!
//! - missing ticket lookup mapping to `tefBAD_LEDGER`,
//! - owner-directory removal before any account-root mutation,
//! - missing owner-directory removal or account-root lookup mapping to
//!   `tefBAD_LEDGER`,
//! - clearing `sfTicketCount` when the count drops from `1` to absent,
//! - otherwise decrementing `sfTicketCount`,
//! - adjusting owner count before erasing the ticket,
//! - and returning `tesSUCCESS` after the ticket erase.

use protocol::Ter;

pub trait TransactorTicketDeleteTicket {
    fn owner_page(&self) -> u64;
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_ticket_delete<AccountId, TicketIndex, Ticket, AccountState>(
    account: &AccountId,
    ticket_index: &TicketIndex,
    ticket: Option<Ticket>,
    dir_remove: impl FnOnce(&AccountId, u64, &TicketIndex, bool) -> bool,
    account_state: Option<&mut AccountState>,
    ticket_count: impl FnOnce(&AccountState) -> Option<u32>,
    set_ticket_count: impl FnOnce(&mut AccountState, Option<u32>),
    adjust_owner_count: impl FnOnce(&mut AccountState, i32),
    erase_ticket: impl FnOnce(Ticket),
) -> Ter
where
    Ticket: TransactorTicketDeleteTicket,
{
    let Some(ticket) = ticket else {
        return Ter::TEF_BAD_LEDGER;
    };

    if !dir_remove(account, ticket.owner_page(), ticket_index, true) {
        return Ter::TEF_BAD_LEDGER;
    }

    let Some(account_state) = account_state else {
        return Ter::TEF_BAD_LEDGER;
    };

    let Some(current_ticket_count) = ticket_count(account_state) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let next_ticket_count = if current_ticket_count == 1 {
        None
    } else {
        Some(current_ticket_count.wrapping_sub(1))
    };
    set_ticket_count(account_state, next_ticket_count);

    adjust_owner_count(account_state, -1);
    erase_ticket(ticket);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use super::{TransactorTicketDeleteTicket, run_transactor_ticket_delete};

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
    fn transactor_ticket_delete_returns_bad_ledger_when_ticket_lookup_misses() {
        let result = run_transactor_ticket_delete(
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

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
    }

    #[test]
    fn transactor_ticket_delete_returns_bad_ledger_on_dir_remove_or_account_miss() {
        let mut account = Account {
            ticket_count: Some(3),
            owner_count: 8,
        };

        let dir_remove_failed = run_transactor_ticket_delete(
            &"alice",
            &"ticket-1",
            Some(Ticket {
                owner_page: 22,
                id: "ticket-1",
            }),
            |_, _, _, _| false,
            Some(&mut account),
            |_| panic!("failed dirRemove should skip ticket count"),
            |_, _| panic!("failed dirRemove should skip ticket-count writes"),
            |_, _| panic!("failed dirRemove should skip owner-count adjustment"),
            |_| panic!("failed dirRemove should skip erase"),
        );

        let account_missing = run_transactor_ticket_delete(
            &"alice",
            &"ticket-1",
            Some(Ticket {
                owner_page: 22,
                id: "ticket-1",
            }),
            |account, page, ticket_index, keep_root| {
                assert_eq!(*account, "alice");
                assert_eq!(page, 22);
                assert_eq!(*ticket_index, "ticket-1");
                assert!(keep_root);
                true
            },
            None::<&mut Account>,
            |_| panic!("missing account should skip ticket count"),
            |_, _| panic!("missing account should skip ticket-count writes"),
            |_, _| panic!("missing account should skip owner-count adjustment"),
            |_| panic!("missing account should skip erase"),
        );

        assert_eq!(dir_remove_failed, Ter::TEF_BAD_LEDGER);
        assert_eq!(account_missing, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn transactor_ticket_delete_returns_bad_ledger_when_ticket_count_is_missing() {
        let mut account = Account {
            ticket_count: None,
            owner_count: 8,
        };

        let result = run_transactor_ticket_delete(
            &"alice",
            &"ticket-1",
            Some(Ticket {
                owner_page: 22,
                id: "ticket-1",
            }),
            |_, _, _, _| true,
            Some(&mut account),
            |account| account.ticket_count,
            |_, _| panic!("missing count should skip ticket-count writes"),
            |_, _| panic!("missing count should skip owner-count adjustment"),
            |_| panic!("missing count should skip erase"),
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn transactor_ticket_delete_clears_or_decrements_count_before_owner_adjust_and_erase() {
        let clear_trace = RefCell::new(Vec::new());
        let erased_clear = Cell::new("");
        let mut one_ticket_account = Account {
            ticket_count: Some(1),
            owner_count: 8,
        };

        let clear_result = run_transactor_ticket_delete(
            &"alice",
            &"ticket-1",
            Some(Ticket {
                owner_page: 22,
                id: "ticket-1",
            }),
            |_, _, _, _| {
                clear_trace.borrow_mut().push("dir_remove");
                true
            },
            Some(&mut one_ticket_account),
            |account| {
                clear_trace.borrow_mut().push("ticket_count");
                account.ticket_count
            },
            |account, next| {
                clear_trace.borrow_mut().push("set_ticket_count");
                account.ticket_count = next;
            },
            |account, delta| {
                clear_trace.borrow_mut().push("adjust_owner_count");
                account.owner_count += delta;
            },
            |ticket| {
                clear_trace.borrow_mut().push("erase_ticket");
                erased_clear.set(ticket.id);
            },
        );

        let decrement_trace = RefCell::new(Vec::new());
        let mut many_ticket_account = Account {
            ticket_count: Some(3),
            owner_count: 8,
        };

        let decrement_result = run_transactor_ticket_delete(
            &"alice",
            &"ticket-2",
            Some(Ticket {
                owner_page: 33,
                id: "ticket-2",
            }),
            |_, _, _, _| {
                decrement_trace.borrow_mut().push("dir_remove");
                true
            },
            Some(&mut many_ticket_account),
            |account| {
                decrement_trace.borrow_mut().push("ticket_count");
                account.ticket_count
            },
            |account, next| {
                decrement_trace.borrow_mut().push("set_ticket_count");
                account.ticket_count = next;
            },
            |account, delta| {
                decrement_trace.borrow_mut().push("adjust_owner_count");
                account.owner_count += delta;
            },
            |_| decrement_trace.borrow_mut().push("erase_ticket"),
        );

        assert_eq!(clear_result, Ter::TES_SUCCESS);
        assert_eq!(decrement_result, Ter::TES_SUCCESS);
        assert_eq!(one_ticket_account.ticket_count, None);
        assert_eq!(many_ticket_account.ticket_count, Some(2));
        assert_eq!(one_ticket_account.owner_count, 7);
        assert_eq!(many_ticket_account.owner_count, 7);
        assert_eq!(erased_clear.get(), "ticket-1");
        assert_eq!(
            clear_trace.into_inner(),
            vec![
                "dir_remove",
                "ticket_count",
                "set_ticket_count",
                "adjust_owner_count",
                "erase_ticket"
            ]
        );
        assert_eq!(
            decrement_trace.into_inner(),
            vec![
                "dir_remove",
                "ticket_count",
                "set_ticket_count",
                "adjust_owner_count",
                "erase_ticket"
            ]
        );
    }
}
