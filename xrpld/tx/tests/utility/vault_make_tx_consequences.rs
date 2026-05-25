//! Integration tests that pin the vault-family `makeTxConsequences` entrypoint
//! shape to the current C++ `applySteps.cpp` behavior.

use protocol::{SeqProxy, TxType};
use tx::{
    HasTxnType, TxConsequences, UnknownTransactionType,
    run_vault_make_tx_consequences_for_txn_source, run_vault_make_tx_consequences_for_txn_type,
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
fn vault_make_tx_consequences_keeps_normal_constructor_for_current_vault_subset() {
    for txn_type in [
        TxType::VAULT_CREATE,
        TxType::VAULT_SET,
        TxType::VAULT_DELETE,
        TxType::VAULT_DEPOSIT,
        TxType::VAULT_WITHDRAW,
        TxType::VAULT_CLAWBACK,
    ] {
        let observed =
            run_vault_make_tx_consequences_for_txn_type(txn_type, 17, SeqProxy::sequence(5))
                .expect("vault type should classify");

        assert_eq!(observed, TxConsequences::new(17, SeqProxy::sequence(5)));
    }
}

#[test]
fn vault_make_tx_consequences_preserves_unknown_fallback() {
    let observed =
        run_vault_make_tx_consequences_for_txn_type(TxType::PAYMENT, 17, SeqProxy::ticket(6));

    assert_eq!(observed, Err(UnknownTransactionType::new(TxType::PAYMENT)));
}

#[test]
fn vault_make_tx_consequences_source_wrapper_uses_txn_type_from_source() {
    let observed = run_vault_make_tx_consequences_for_txn_source(
        &TestTx {
            txn_type: TxType::VAULT_DEPOSIT,
        },
        19,
        SeqProxy::ticket(8),
    )
    .expect("vault type should classify");

    assert_eq!(observed, TxConsequences::new(19, SeqProxy::ticket(8)));
}
