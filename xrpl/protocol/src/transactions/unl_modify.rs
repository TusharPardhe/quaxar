use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UNLModify {
    base: crate::TransactionBase,
}

impl UNLModify {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(102);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for UNLModify".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_unl_modify_disabling(&self) -> u8 {
        self.base
            .as_sttx()
            .get_field_u8(crate::get_field_by_symbol("sfUNLModifyDisabling"))
    }

    pub fn get_ledger_sequence(&self) -> u32 {
        self.base
            .as_sttx()
            .get_field_u32(crate::get_field_by_symbol("sfLedgerSequence"))
    }

    pub fn get_unl_modify_validator(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(crate::get_field_by_symbol("sfUNLModifyValidator"))
    }
}

impl Deref for UNLModify {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UNLModifyBuilder {
    base: crate::TransactionBuilderBase,
}

impl UNLModifyBuilder {
    pub fn new(
        account: crate::AccountID,
        unl_modify_disabling: u8,
        ledger_sequence: u32,
        unl_modify_validator: impl AsRef<[u8]>,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(UNLModify::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_unl_modify_disabling(unl_modify_disabling);
        builder = builder.set_ledger_sequence(ledger_sequence);
        builder = builder.set_unl_modify_validator(unl_modify_validator);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != UNLModify::TX_TYPE {
            return Err("Invalid transaction type for UNLModifyBuilder".to_owned());
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

    pub fn set_unl_modify_disabling(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfUNLModifyDisabling"), value);
        self
    }

    pub fn set_ledger_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLedgerSequence"), value);
        self
    }

    pub fn set_unl_modify_validator(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfUNLModifyValidator"),
            value.as_ref(),
        );
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<UNLModify, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(UNLModify::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
