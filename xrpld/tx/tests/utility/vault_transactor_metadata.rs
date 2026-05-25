//! Integration tests that pin the shared static vault transactor metadata to
//! the current C++ headers and `applySteps.cpp` vault transaction subset.

use protocol::TxType;
use tx::{
    HasTxnType, TxConsequencesCategory, UnknownTransactionType, VaultTxnType,
    classify_vault_txn_type, run_with_vault_txn_type_key, run_with_vault_txn_type_source,
    vault_txn_consequences_category,
};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_transactor_metadata_classifies_all_current_vault_tx_types() {
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
fn vault_transactor_metadata_rejects_non_vault_tx_types_subset() {
    assert_eq!(
        run_with_vault_txn_type_key(TxType::BATCH, |txn_type| txn_type),
        Err(UnknownTransactionType::new(TxType::BATCH))
    );
}

#[test]
fn vault_transactor_metadata_reads_txn_type_from_source_subset() {
    let result = run_with_vault_txn_type_source(
        &TestTx {
            txn_type: TxType::VAULT_WITHDRAW,
        },
        |txn_type| txn_type,
    );

    assert_eq!(result, Ok(VaultTxnType::Withdraw));
}

#[test]
fn vault_transactor_metadata_keeps_normal_consequences_factory_for_all_vault_types_headers() {
    for txn_type in [
        VaultTxnType::Create,
        VaultTxnType::Set,
        VaultTxnType::Delete,
        VaultTxnType::Deposit,
        VaultTxnType::Withdraw,
        VaultTxnType::Clawback,
    ] {
        assert_eq!(
            vault_txn_consequences_category(txn_type),
            TxConsequencesCategory::Normal
        );
    }
}
