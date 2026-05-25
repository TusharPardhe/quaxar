use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    base: crate::TransactionBase,
}

impl Batch {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(71);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for Batch".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_raw_transactions(&self) -> crate::STArray {
        self.base
            .as_sttx()
            .get_field_array(crate::get_field_by_symbol("sfRawTransactions"))
    }

    pub fn get_batch_signers(&self) -> Option<crate::STArray> {
        self.has_batch_signers().then(|| {
            self.base
                .as_sttx()
                .get_field_array(crate::get_field_by_symbol("sfBatchSigners"))
        })
    }

    pub fn has_batch_signers(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfBatchSigners"))
    }
}

impl Deref for Batch {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchBuilder {
    base: crate::TransactionBuilderBase,
}

impl BatchBuilder {
    pub fn new(
        account: crate::AccountID,
        raw_transactions: crate::STArray,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(Batch::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_raw_transactions(raw_transactions);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Batch::TX_TYPE {
            return Err("Invalid transaction type for BatchBuilder".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBuilderBase::from_tx(tx),
        })
    }

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base.set_account(value);
        self
    }

    pub fn set_fee(mut self, value: crate::STAmount) -> Self {
        self.base.set_fee(value);
        self
    }

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base.set_sequence(value);
        self
    }

    pub fn set_ticket_sequence(mut self, value: u32) -> Self {
        self.base.set_ticket_sequence(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_source_tag(mut self, value: u32) -> Self {
        self.base.set_source_tag(value);
        self
    }

    pub fn set_last_ledger_sequence(mut self, value: u32) -> Self {
        self.base.set_last_ledger_sequence(value);
        self
    }

    pub fn set_account_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_account_txn_id(value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_previous_txn_id(value);
        self
    }

    pub fn set_operation_limit(mut self, value: u32) -> Self {
        self.base.set_operation_limit(value);
        self
    }

    pub fn set_memos(mut self, value: crate::STArray) -> Self {
        self.base.set_memos(value);
        self
    }

    pub fn set_signers(mut self, value: crate::STArray) -> Self {
        self.base.set_signers(value);
        self
    }

    pub fn set_network_id(mut self, value: u32) -> Self {
        self.base.set_network_id(value);
        self
    }

    pub fn set_delegate(mut self, value: crate::AccountID) -> Self {
        self.base.set_delegate(value);
        self
    }

    pub fn get_st_object(&self) -> &crate::STObject {
        self.base.object()
    }

    pub fn set_raw_transactions(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfRawTransactions"), value);
        self
    }

    pub fn set_batch_signers(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfBatchSigners"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<Batch, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(Batch::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
