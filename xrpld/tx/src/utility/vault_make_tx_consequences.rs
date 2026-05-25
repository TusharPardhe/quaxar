//! Vault-family `makeTxConsequences` entrypoint shape that the reference implementation reaches
//! through the transaction dispatch layer.
//!
//! This ports the deterministic vault-family routing that matters here:
//!
//! - all current vault transaction types stay on the normal/base constructor,
//! - source and explicit-type wrappers share the same vault classifier,
//! - and unknown transaction types still map to `UnknownTransactionType`.

use protocol::{SeqProxy, TxType};

use crate::{
    HasTxnType, TxConsequences, UnknownTransactionType, run_with_vault_txn_type_key, txn_type_of,
};

pub fn run_vault_make_tx_consequences_for_txn_type(
    txn_type: TxType,
    fee_drops: u64,
    seq_proxy: SeqProxy,
) -> Result<TxConsequences, UnknownTransactionType<TxType>> {
    run_with_vault_txn_type_key(txn_type, |_| TxConsequences::new(fee_drops, seq_proxy))
}

pub fn run_vault_make_tx_consequences_for_txn_source<Tx: HasTxnType + ?Sized>(
    tx: &Tx,
    fee_drops: u64,
    seq_proxy: SeqProxy,
) -> Result<TxConsequences, UnknownTransactionType<TxType>> {
    run_vault_make_tx_consequences_for_txn_type(txn_type_of(tx), fee_drops, seq_proxy)
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, TxType};

    use super::{
        run_vault_make_tx_consequences_for_txn_source, run_vault_make_tx_consequences_for_txn_type,
    };
    use crate::{HasTxnType, TxConsequences, UnknownTransactionType};

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn vault_make_tx_consequences_keeps_normal_constructor_for_all_vault_types() {
        for txn_type in [
            TxType::VAULT_CREATE,
            TxType::VAULT_SET,
            TxType::VAULT_DELETE,
            TxType::VAULT_DEPOSIT,
            TxType::VAULT_WITHDRAW,
            TxType::VAULT_CLAWBACK,
        ] {
            let observed =
                run_vault_make_tx_consequences_for_txn_type(txn_type, 11, SeqProxy::sequence(7))
                    .expect("vault type should classify");

            assert_eq!(observed, TxConsequences::new(11, SeqProxy::sequence(7)));
        }
    }

    #[test]
    fn vault_make_tx_consequences_preserves_unknown_fallback() {
        let observed =
            run_vault_make_tx_consequences_for_txn_type(TxType::PAYMENT, 11, SeqProxy::ticket(4));

        assert_eq!(observed, Err(UnknownTransactionType::new(TxType::PAYMENT)));
    }

    #[test]
    fn vault_make_tx_consequences_source_wrapper_reads_txn_type_from_source() {
        let observed = run_vault_make_tx_consequences_for_txn_source(
            &TestTx {
                txn_type: TxType::VAULT_WITHDRAW,
            },
            13,
            SeqProxy::ticket(9),
        )
        .expect("vault type should classify");

        assert_eq!(observed, TxConsequences::new(13, SeqProxy::ticket(9)));
    }
}
