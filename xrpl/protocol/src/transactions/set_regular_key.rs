use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRegularKey {
    base: crate::TransactionBase,
}

impl SetRegularKey {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(5);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for SetRegularKey".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_regular_key(&self) -> Option<crate::AccountID> {
        self.has_regular_key().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfRegularKey"))
        })
    }

    pub fn has_regular_key(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfRegularKey"))
    }
}

impl Deref for SetRegularKey {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRegularKeyBuilder {
    base: crate::TransactionBuilderBase,
}

impl SetRegularKeyBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(
                SetRegularKey::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        }
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != SetRegularKey::TX_TYPE {
            return Err("Invalid transaction type for SetRegularKeyBuilder".to_owned());
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

    pub fn set_regular_key(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfRegularKey"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<SetRegularKey, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(SetRegularKey::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
