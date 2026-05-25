//! System-family `invokePreclaim(...)` dispatch shell above the landed `Change`,
//! `Batch`, `TicketCreate`, and `LedgerStateFix` preclaim helpers.
//!
//! This ports the deterministic branch selection that the reference implementation performs
//! for:
//!
//! - `ttAMENDMENT`, `ttFEE`, and `ttUNL_MODIFY` via `Change::preclaim()`,
//! - `ttBATCH` via its custom sign-checking sequence,
//! - `ttTICKET_CREATE` via `TicketCreate::preclaim()`,
//! - `ttLEDGER_STATE_FIX` via `LedgerStateFix::preclaim()`,
//! - and the current `temUNKNOWN` fallback for everything else.

use protocol::{NotTec, Ter, TxType};

use crate::ledger_state_fix::LedgerStateFixType;
use crate::{
    ChangePreclaimFacts, HasTxnType, TicketCreatePreclaimFacts, UnknownTransactionType,
    run_change_preclaim, run_ledger_state_fix_preclaim, run_ticket_create_preclaim,
    run_transactor_invoke_preclaim,
};

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preclaim_for_txn_type<Fee>(
    account_is_zero: bool,
    txn_type: TxType,
    change_preclaim_facts: ChangePreclaimFacts,
    ticket_create_preclaim_facts: TicketCreatePreclaimFacts,
    ledger_state_fix_type: LedgerStateFixType,
    ledger_state_fix_owner_exists: bool,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    check_batch_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    payment_preclaim: impl FnOnce() -> Ter,
    offer_create_preclaim: impl FnOnce() -> Ter,
    offer_cancel_preclaim: impl FnOnce() -> Ter,
    trust_set_preclaim: impl FnOnce() -> Ter,
    nft_create_offer_preclaim: impl FnOnce() -> Ter,
    nft_cancel_offer_preclaim: impl FnOnce() -> Ter,
    nft_accept_offer_preclaim: impl FnOnce() -> Ter,
) -> Result<Ter, UnknownTransactionType<TxType>> {
    match txn_type {
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            || run_change_preclaim(txn_type, change_preclaim_facts),
        )),
        TxType::BATCH => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            || {
                let ret = check_sign();
                if ret != Ter::TES_SUCCESS {
                    return ret;
                }

                check_batch_sign()
            },
            calculate_base_fee,
            check_fee,
            || Ter::TES_SUCCESS,
        )),
        TxType::TICKET_CREATE => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            || run_ticket_create_preclaim(ticket_create_preclaim_facts),
        )),
        TxType::LEDGER_STATE_FIX => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            || run_ledger_state_fix_preclaim(ledger_state_fix_type, ledger_state_fix_owner_exists),
        )),
        TxType::PAYMENT => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            payment_preclaim,
        )),
        TxType::OFFER_CREATE => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            offer_create_preclaim,
        )),
        TxType::OFFER_CANCEL => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            offer_cancel_preclaim,
        )),
        TxType::TRUST_SET => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            trust_set_preclaim,
        )),
        TxType::NFTOKEN_CREATE_OFFER => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            nft_create_offer_preclaim,
        )),
        TxType::NFTOKEN_CANCEL_OFFER => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            nft_cancel_offer_preclaim,
        )),
        TxType::NFTOKEN_ACCEPT_OFFER => Ok(run_transactor_invoke_preclaim(
            account_is_zero,
            check_seq_proxy,
            check_prior_tx_and_last_ledger,
            check_permission,
            check_sign,
            calculate_base_fee,
            check_fee,
            nft_accept_offer_preclaim,
        )),
        _ => Err(UnknownTransactionType::new(txn_type)),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_preclaim_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    account_is_zero: bool,
    tx: &Tx,
    change_preclaim_facts: ChangePreclaimFacts,
    ticket_create_preclaim_facts: TicketCreatePreclaimFacts,
    ledger_state_fix_type: LedgerStateFixType,
    ledger_state_fix_owner_exists: bool,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    check_batch_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
) -> Result<Ter, UnknownTransactionType<TxType>> {
    run_system_invoke_preclaim_for_txn_type(
        account_is_zero,
        tx.txn_type(),
        change_preclaim_facts,
        ticket_create_preclaim_facts,
        ledger_state_fix_type,
        ledger_state_fix_owner_exists,
        check_seq_proxy,
        check_prior_tx_and_last_ledger,
        check_permission,
        check_sign,
        check_batch_sign,
        calculate_base_fee,
        check_fee,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, TxType};

    use super::{
        run_system_invoke_preclaim_for_txn_source, run_system_invoke_preclaim_for_txn_type,
    };
    use crate::ledger_state_fix::LedgerStateFixType;
    use crate::{
        ChangePreclaimFacts, HasTxnType, TicketCreatePreclaimFacts, UnknownTransactionType,
    };

    struct StubTx {
        txn_type: TxType,
    }

    impl HasTxnType for StubTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn system_invoke_preclaim_uses_change_path_for_change_family() {
        let result = run_system_invoke_preclaim_for_txn_type(
            true,
            TxType::AMENDMENT,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts::default(),
            LedgerStateFixType::NfTokenPageLink,
            true,
            || panic!("zero-account change path should skip seq"),
            || panic!("zero-account change path should skip prior"),
            || panic!("zero-account change path should skip permission"),
            || panic!("zero-account change path should skip sign"),
            || panic!("change path should skip batch-sign"),
            || panic!("zero-account change path should skip base-fee"),
            |_| panic!("zero-account change path should skip fee"),
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TES_SUCCESS));
    }

    #[test]
    fn system_invoke_preclaim_preserves_batch_sign_order() {
        let trace = RefCell::new(Vec::new());

        let result = run_system_invoke_preclaim_for_txn_type(
            false,
            TxType::BATCH,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts::default(),
            LedgerStateFixType::NfTokenPageLink,
            true,
            || {
                trace.borrow_mut().push("seq");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("prior");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("permission");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("sign");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("batch-sign");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("base-fee");
                20_u64
            },
            |fee| {
                trace.borrow_mut().push("fee");
                assert_eq!(fee, 20);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TES_SUCCESS));
        assert_eq!(
            trace.into_inner(),
            vec![
                "seq",
                "prior",
                "permission",
                "sign",
                "batch-sign",
                "base-fee",
                "fee"
            ]
        );
    }

    #[test]
    fn system_invoke_preclaim_preserves_batch_sign_failure_shortcut() {
        let result = run_system_invoke_preclaim_for_txn_type(
            false,
            TxType::BATCH,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts::default(),
            LedgerStateFixType::NfTokenPageLink,
            true,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TEF_BAD_SIGNATURE,
            || panic!("batch-sign failure should skip base-fee"),
            |_| panic!("batch-sign failure should skip fee"),
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ok(Ter::TEF_BAD_SIGNATURE));
    }

    #[test]
    fn system_invoke_preclaim_routes_ticket_create_and_ledger_fix_helpers() {
        let ticket = run_system_invoke_preclaim_for_txn_type(
            false,
            TxType::TICKET_CREATE,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts {
                account_exists: true,
                current_ticket_count: 10,
                requested_ticket_count: 2,
                consumes_ticket_sequence: false,
            },
            LedgerStateFixType::NfTokenPageLink,
            true,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || panic!("ticket-create path should skip batch-sign"),
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let ledger_fix = run_system_invoke_preclaim_for_txn_type(
            false,
            TxType::LEDGER_STATE_FIX,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts::default(),
            LedgerStateFixType::NfTokenPageLink,
            false,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || panic!("ledger-fix path should skip batch-sign"),
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(ticket, Ok(Ter::TES_SUCCESS));
        assert_eq!(ledger_fix, Ok(Ter::TEC_OBJECT_NOT_FOUND));
    }

    #[test]
    fn system_invoke_preclaim_source_wrapper_preserves_unknowns_subset() {
        let tx = StubTx {
            txn_type: TxType::ESCROW_CREATE,
        };

        let result = run_system_invoke_preclaim_for_txn_source(
            false,
            &tx,
            ChangePreclaimFacts::default(),
            TicketCreatePreclaimFacts::default(),
            LedgerStateFixType::NfTokenPageLink,
            true,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| Ter::TES_SUCCESS,
        );

        assert_eq!(
            result,
            Err(UnknownTransactionType::new(TxType::ESCROW_CREATE))
        );
    }
}
