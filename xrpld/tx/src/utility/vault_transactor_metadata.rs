//! Shared static vault transactor metadata that the reference implementation vault classes
//! expose through headers and the transaction dispatch layer.
//!
//! This ports the exact current behavior around:
//!
//! - selecting the six vault transaction types from the larger `TxType` set,
//! - returning `UnknownTransactionType` for non-vault transaction types,
//! - and reporting the current header-declared `ConsequencesFactory{Normal}`
//!   role for all vault transactors.

use protocol::TxType;

use crate::{HasTxnType, TxConsequencesCategory, UnknownTransactionType, txn_type_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VaultTxnType {
    Create,
    Set,
    Delete,
    Deposit,
    Withdraw,
    Clawback,
}

pub fn classify_vault_txn_type(txn_type: TxType) -> Option<VaultTxnType> {
    if txn_type == TxType::VAULT_CREATE {
        Some(VaultTxnType::Create)
    } else if txn_type == TxType::VAULT_SET {
        Some(VaultTxnType::Set)
    } else if txn_type == TxType::VAULT_DELETE {
        Some(VaultTxnType::Delete)
    } else if txn_type == TxType::VAULT_DEPOSIT {
        Some(VaultTxnType::Deposit)
    } else if txn_type == TxType::VAULT_WITHDRAW {
        Some(VaultTxnType::Withdraw)
    } else if txn_type == TxType::VAULT_CLAWBACK {
        Some(VaultTxnType::Clawback)
    } else {
        None
    }
}

pub fn run_with_vault_txn_type_key<R>(
    txn_type: TxType,
    dispatch: impl FnOnce(VaultTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    match classify_vault_txn_type(txn_type) {
        Some(vault_txn_type) => Ok(dispatch(vault_txn_type)),
        None => Err(UnknownTransactionType::new(txn_type)),
    }
}

pub fn run_with_vault_txn_type_source<Tx: HasTxnType + ?Sized, R>(
    tx: &Tx,
    dispatch: impl FnOnce(VaultTxnType) -> R,
) -> Result<R, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type_of(tx), dispatch)
}

pub const fn vault_txn_consequences_category(_: VaultTxnType) -> TxConsequencesCategory {
    TxConsequencesCategory::Normal
}

#[cfg(test)]
mod tests {
    use protocol::TxType;

    use super::{
        VaultTxnType, classify_vault_txn_type, run_with_vault_txn_type_key,
        run_with_vault_txn_type_source, vault_txn_consequences_category,
    };
    use crate::{HasTxnType, TxConsequencesCategory, UnknownTransactionType};

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn vault_txn_type_classifier_matches_current_cpp_vault_tx_set() {
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_CREATE),
            Some(VaultTxnType::Create)
        );
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_SET),
            Some(VaultTxnType::Set)
        );
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_DELETE),
            Some(VaultTxnType::Delete)
        );
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_DEPOSIT),
            Some(VaultTxnType::Deposit)
        );
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_WITHDRAW),
            Some(VaultTxnType::Withdraw)
        );
        assert_eq!(
            classify_vault_txn_type(TxType::VAULT_CLAWBACK),
            Some(VaultTxnType::Clawback)
        );
    }

    #[test]
    fn vault_txn_type_classifier_rejects_non_vault_transactions_switch_subset() {
        assert_eq!(classify_vault_txn_type(TxType::BATCH), None);
        assert_eq!(classify_vault_txn_type(TxType::PAYMENT), None);
    }

    #[test]
    fn vault_txn_type_key_wrapper_dispatches_and_returns_unknown_subset() {
        let selected =
            run_with_vault_txn_type_key(TxType::VAULT_CLAWBACK, |txn_type| txn_type).unwrap();
        let unknown = run_with_vault_txn_type_key(TxType::PAYMENT, |txn_type| txn_type);

        assert_eq!(selected, VaultTxnType::Clawback);
        assert_eq!(unknown, Err(UnknownTransactionType::new(TxType::PAYMENT)));
    }

    #[test]
    fn vault_txn_type_source_wrapper_reads_txn_type_from_source_subset() {
        let result = run_with_vault_txn_type_source(
            &TestTx {
                txn_type: TxType::VAULT_DEPOSIT,
            },
            |txn_type| txn_type,
        );

        assert_eq!(result, Ok(VaultTxnType::Deposit));
    }

    #[test]
    fn vault_transactor_consequences_factory_is_normal_for_all_vault_types_headers() {
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Create),
            TxConsequencesCategory::Normal
        );
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Set),
            TxConsequencesCategory::Normal
        );
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Delete),
            TxConsequencesCategory::Normal
        );
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Deposit),
            TxConsequencesCategory::Normal
        );
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Withdraw),
            TxConsequencesCategory::Normal
        );
        assert_eq!(
            vault_txn_consequences_category(VaultTxnType::Clawback),
            TxConsequencesCategory::Normal
        );
    }
}
