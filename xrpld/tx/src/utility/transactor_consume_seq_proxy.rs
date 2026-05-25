//! Current Rust helper mirroring `Transactor::consumeSeqProxy(...)`.
//!
//! This module preserves the exact current outer behavior:
//!
//! - increment the account sequence to `seq + 1` when the transaction is using
//!   a sequence number,
//! - otherwise delegate to the ticket path unchanged.

use protocol::{SeqProxy, Ter};

pub fn run_transactor_consume_seq_proxy<AccountState, SetAccountSequence, TicketDelete>(
    seq_proxy: SeqProxy,
    account_state: &mut AccountState,
    mut set_account_sequence: SetAccountSequence,
    ticket_delete: TicketDelete,
) -> Ter
where
    SetAccountSequence: FnMut(&mut AccountState, u32),
    TicketDelete: FnOnce() -> Ter,
{
    if seq_proxy.is_seq() {
        set_account_sequence(account_state, seq_proxy.value() + 1);
        return Ter::TES_SUCCESS;
    }

    ticket_delete()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{SeqProxy, Ter};

    use super::run_transactor_consume_seq_proxy;
    use crate::{TransactorTicketDeleteTicket, run_transactor_ticket_delete};

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
    fn transactor_consume_seq_proxy_increments_sequence_numbers() {
        let ticket_delete_called = Cell::new(false);
        let mut account = Account { sequence: 7 };

        let result = run_transactor_consume_seq_proxy(
            SeqProxy::sequence(11),
            &mut account,
            |account, next_sequence| account.sequence = next_sequence,
            || {
                ticket_delete_called.set(true);
                Ter::TEF_BAD_LEDGER
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(account, Account { sequence: 12 });
        assert!(!ticket_delete_called.get());
    }

    #[test]
    fn transactor_consume_seq_proxy_delegates_ticket_path() {
        let mut account = Account { sequence: 7 };

        let result = run_transactor_consume_seq_proxy(
            SeqProxy::ticket(22),
            &mut account,
            |account, next_sequence| account.sequence = next_sequence,
            || Ter::TEF_NO_TICKET,
        );

        assert_eq!(result, Ter::TEF_NO_TICKET);
        assert_eq!(account, Account { sequence: 7 });
    }

    #[test]
    fn transactor_consume_seq_proxy_composes_with_ticket_delete_helper() {
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
}
