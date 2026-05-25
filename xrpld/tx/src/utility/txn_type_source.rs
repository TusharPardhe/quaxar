//! Callers that can expose a protocol-owned transaction type key without a
//! full `STTx` port.
//!
//! This keeps the next `applySteps` and transactor-adjacent layers honest:
//! callers can provide a real `protocol::TxType`.

use protocol::TxType;

pub trait HasTxnType {
    fn txn_type(&self) -> TxType;
}

impl HasTxnType for TxType {
    fn txn_type(&self) -> TxType {
        *self
    }
}

impl<T: HasTxnType + ?Sized> HasTxnType for &T {
    fn txn_type(&self) -> TxType {
        (*self).txn_type()
    }
}

pub fn txn_type_of<T: HasTxnType + ?Sized>(value: &T) -> TxType {
    value.txn_type()
}

#[cfg(test)]
mod tests {
    use protocol::TxType;

    use super::{HasTxnType, txn_type_of};

    struct StubTxn {
        txn_type: TxType,
    }

    impl HasTxnType for StubTxn {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn tx_type_source_reads_protocol_key_from_stub_transaction() {
        let txn = StubTxn {
            txn_type: TxType::PAYMENT,
        };

        assert_eq!(txn_type_of(&txn), TxType::PAYMENT);
    }

    #[test]
    fn tx_type_source_allows_tx_type_itself_as_the_key_provider() {
        assert_eq!(txn_type_of(&TxType::HOOK_SET), TxType::HOOK_SET);
    }
}
