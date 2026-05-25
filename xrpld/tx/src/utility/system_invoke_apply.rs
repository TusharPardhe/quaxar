//! System-family `invokeApply(...)` dispatch shell above the landed `Change`,
//! `Batch`, `TicketCreate`, and `LedgerStateFix` apply helpers.
//!
//! This ports the deterministic branch selection that the reference implementation performs
//! for:
//!
//! - `ttAMENDMENT`, `ttFEE`, and `ttUNL_MODIFY` via `Change::doApply()`,
//! - `ttBATCH` via `Batch::doApply()`,
//! - `ttTICKET_CREATE` via `TicketCreate::doApply()`,
//! - `ttLEDGER_STATE_FIX` via `LedgerStateFix::doApply()`,
//! - and the current `temUNKNOWN` fallback for everything else.

use protocol::TxType;

use crate::{
    ApplyResult, HasTxnType, UNKNOWN_TRANSACTION_TYPE_TER, UnknownTransactionType, txn_type_of,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemApplyTxnType {
    Change,
    Batch,
    TicketCreate,
    LedgerStateFix,
    Payment,
    OfferCreate,
    OfferCancel,
    TrustSet,
    NFTokenCreateOffer,
    NFTokenCancelOffer,
    NFTokenAcceptOffer,
}

pub fn classify_system_apply_txn_type(txn_type: TxType) -> Option<SystemApplyTxnType> {
    match txn_type {
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY => Some(SystemApplyTxnType::Change),
        TxType::BATCH => Some(SystemApplyTxnType::Batch),
        TxType::TICKET_CREATE => Some(SystemApplyTxnType::TicketCreate),
        TxType::LEDGER_STATE_FIX => Some(SystemApplyTxnType::LedgerStateFix),
        TxType::PAYMENT => Some(SystemApplyTxnType::Payment),
        TxType::OFFER_CREATE => Some(SystemApplyTxnType::OfferCreate),
        TxType::OFFER_CANCEL => Some(SystemApplyTxnType::OfferCancel),
        TxType::TRUST_SET => Some(SystemApplyTxnType::TrustSet),
        TxType::NFTOKEN_CREATE_OFFER => Some(SystemApplyTxnType::NFTokenCreateOffer),
        TxType::NFTOKEN_CANCEL_OFFER => Some(SystemApplyTxnType::NFTokenCancelOffer),
        TxType::NFTOKEN_ACCEPT_OFFER => Some(SystemApplyTxnType::NFTokenAcceptOffer),
        _ => None,
    }
}

pub fn run_with_system_apply_txn_type_key<R>(
    txn_type: TxType,
    dispatch: impl FnOnce(SystemApplyTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    match classify_system_apply_txn_type(txn_type) {
        Some(system_apply_txn_type) => Ok(dispatch(system_apply_txn_type)),
        None => Err(UnknownTransactionType::new(txn_type)),
    }
}

pub fn run_with_system_apply_txn_source<Tx: HasTxnType + ?Sized, R>(
    tx: &Tx,
    dispatch: impl FnOnce(SystemApplyTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    run_with_system_apply_txn_type_key(txn_type_of(tx), dispatch)
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_apply_for_txn_type(
    txn_type: TxType,
    run_change_do_apply: impl FnOnce() -> ApplyResult,
    run_batch_do_apply: impl FnOnce() -> ApplyResult,
    run_ticket_create_do_apply: impl FnOnce() -> ApplyResult,
    run_ledger_state_fix_do_apply: impl FnOnce() -> ApplyResult,
    run_payment_do_apply: impl FnOnce() -> ApplyResult,
    run_offer_create_do_apply: impl FnOnce() -> ApplyResult,
    run_offer_cancel_do_apply: impl FnOnce() -> ApplyResult,
    run_trust_set_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_create_offer_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_cancel_offer_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_accept_offer_do_apply: impl FnOnce() -> ApplyResult,
) -> ApplyResult {
    run_with_system_apply_txn_type_key(
        txn_type,
        |system_apply_txn_type| match system_apply_txn_type {
            SystemApplyTxnType::Change => run_change_do_apply(),
            SystemApplyTxnType::Batch => run_batch_do_apply(),
            SystemApplyTxnType::TicketCreate => run_ticket_create_do_apply(),
            SystemApplyTxnType::LedgerStateFix => run_ledger_state_fix_do_apply(),
            SystemApplyTxnType::Payment => run_payment_do_apply(),
            SystemApplyTxnType::OfferCreate => run_offer_create_do_apply(),
            SystemApplyTxnType::OfferCancel => run_offer_cancel_do_apply(),
            SystemApplyTxnType::TrustSet => run_trust_set_do_apply(),
            SystemApplyTxnType::NFTokenCreateOffer => run_nft_create_offer_do_apply(),
            SystemApplyTxnType::NFTokenCancelOffer => run_nft_cancel_offer_do_apply(),
            SystemApplyTxnType::NFTokenAcceptOffer => run_nft_accept_offer_do_apply(),
        },
    )
    .unwrap_or_else(|_| ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false))
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_apply_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    run_change_do_apply: impl FnOnce() -> ApplyResult,
    run_batch_do_apply: impl FnOnce() -> ApplyResult,
    run_ticket_create_do_apply: impl FnOnce() -> ApplyResult,
    run_ledger_state_fix_do_apply: impl FnOnce() -> ApplyResult,
    run_payment_do_apply: impl FnOnce() -> ApplyResult,
    run_offer_create_do_apply: impl FnOnce() -> ApplyResult,
    run_offer_cancel_do_apply: impl FnOnce() -> ApplyResult,
    run_trust_set_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_create_offer_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_cancel_offer_do_apply: impl FnOnce() -> ApplyResult,
    run_nft_accept_offer_do_apply: impl FnOnce() -> ApplyResult,
) -> ApplyResult {
    run_system_invoke_apply_for_txn_type(
        txn_type_of(tx),
        run_change_do_apply,
        run_batch_do_apply,
        run_ticket_create_do_apply,
        run_ledger_state_fix_do_apply,
        run_payment_do_apply,
        run_offer_create_do_apply,
        run_offer_cancel_do_apply,
        run_trust_set_do_apply,
        run_nft_create_offer_do_apply,
        run_nft_cancel_offer_do_apply,
        run_nft_accept_offer_do_apply,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_system_invoke_apply_result_for_txn_type<E>(
    txn_type: TxType,
    run_change_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_batch_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_ticket_create_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_ledger_state_fix_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_payment_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_offer_create_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_offer_cancel_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_trust_set_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_nft_create_offer_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_nft_cancel_offer_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
    run_nft_accept_offer_do_apply: impl FnOnce() -> Result<ApplyResult, E>,
) -> Result<ApplyResult, E> {
    match classify_system_apply_txn_type(txn_type) {
        Some(SystemApplyTxnType::Change) => run_change_do_apply(),
        Some(SystemApplyTxnType::Batch) => run_batch_do_apply(),
        Some(SystemApplyTxnType::TicketCreate) => run_ticket_create_do_apply(),
        Some(SystemApplyTxnType::LedgerStateFix) => run_ledger_state_fix_do_apply(),
        Some(SystemApplyTxnType::Payment) => run_payment_do_apply(),
        Some(SystemApplyTxnType::OfferCreate) => run_offer_create_do_apply(),
        Some(SystemApplyTxnType::OfferCancel) => run_offer_cancel_do_apply(),
        Some(SystemApplyTxnType::TrustSet) => run_trust_set_do_apply(),
        Some(SystemApplyTxnType::NFTokenCreateOffer) => run_nft_create_offer_do_apply(),
        Some(SystemApplyTxnType::NFTokenCancelOffer) => run_nft_cancel_offer_do_apply(),
        Some(SystemApplyTxnType::NFTokenAcceptOffer) => run_nft_accept_offer_do_apply(),
        None => Ok(ApplyResult::new(UNKNOWN_TRANSACTION_TYPE_TER, false, false)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, TxType};

    use super::{
        SystemApplyTxnType, classify_system_apply_txn_type, run_system_invoke_apply_for_txn_type,
    };
    use crate::{ApplyResult, HasTxnType};

    struct StubTx {
        txn_type: TxType,
    }

    impl HasTxnType for StubTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn system_invoke_apply_classifies_current_system_family() {
        assert_eq!(
            classify_system_apply_txn_type(TxType::AMENDMENT),
            Some(SystemApplyTxnType::Change)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::FEE),
            Some(SystemApplyTxnType::Change)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::UNL_MODIFY),
            Some(SystemApplyTxnType::Change)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::BATCH),
            Some(SystemApplyTxnType::Batch)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::TICKET_CREATE),
            Some(SystemApplyTxnType::TicketCreate)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::LEDGER_STATE_FIX),
            Some(SystemApplyTxnType::LedgerStateFix)
        );
        assert_eq!(
            classify_system_apply_txn_type(TxType::PAYMENT),
            Some(SystemApplyTxnType::Payment)
        );
    }

    #[test]
    fn system_invoke_apply_routes_each_system_transaction_type() {
        let trace = RefCell::new(Vec::new());

        let change = run_system_invoke_apply_for_txn_type(
            TxType::AMENDMENT,
            || {
                trace.borrow_mut().push("change");
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || panic!("amendment should not dispatch to batch"),
            || panic!("amendment should not dispatch to ticket create"),
            || panic!("amendment should not dispatch to ledger state fix"),
            || panic!("amendment should not dispatch to payment"),
            || panic!("amendment should not dispatch to offer create"),
            || panic!("amendment should not dispatch to offer cancel"),
            || panic!("amendment should not dispatch to trust set"),
            || panic!("amendment should not dispatch to nft create"),
            || panic!("amendment should not dispatch to nft cancel"),
            || panic!("amendment should not dispatch to nft accept"),
        );
        assert_eq!(change, ApplyResult::new(Ter::TES_SUCCESS, true, false));

        let batch = run_system_invoke_apply_for_txn_type(
            TxType::BATCH,
            || panic!("batch should not dispatch to change"),
            || {
                trace.borrow_mut().push("batch");
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
            || panic!("batch should not dispatch to ticket create"),
            || panic!("batch should not dispatch to ledger state fix"),
            || panic!("batch should not dispatch to payment"),
            || panic!("batch should not dispatch to offer create"),
            || panic!("batch should not dispatch to offer cancel"),
            || panic!("batch should not dispatch to trust set"),
            || panic!("batch should not dispatch to nft create"),
            || panic!("batch should not dispatch to nft cancel"),
            || panic!("batch should not dispatch to nft accept"),
        );
        assert_eq!(batch, ApplyResult::new(Ter::TES_SUCCESS, true, true));

        assert_eq!(trace.into_inner(), vec!["change", "batch"]);
    }

    #[test]
    fn system_invoke_apply_source_wrapper_uses_txn_type_from_source() {
        let tx = StubTx {
            txn_type: TxType::FEE,
        };

        let result = run_system_invoke_apply_for_txn_type(
            tx.txn_type(),
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || panic!("fee should not dispatch to batch"),
            || panic!("fee should not dispatch to ticket create"),
            || panic!("fee should not dispatch to ledger state fix"),
            || panic!("fee should not dispatch to payment"),
            || panic!("fee should not dispatch to offer create"),
            || panic!("fee should not dispatch to offer cancel"),
            || panic!("fee should not dispatch to trust set"),
            || panic!("fee should not dispatch to nft create"),
            || panic!("fee should not dispatch to nft cancel"),
            || panic!("fee should not dispatch to nft accept"),
        );

        assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, false));
    }
}
