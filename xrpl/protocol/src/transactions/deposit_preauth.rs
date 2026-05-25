use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauth {
    base: crate::TransactionBase,
}

impl DepositPreauth {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(19);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for DepositPreauth".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_authorize(&self) -> Option<crate::AccountID> {
        self.has_authorize().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfAuthorize"))
        })
    }

    pub fn has_authorize(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAuthorize"))
    }

    pub fn get_unauthorize(&self) -> Option<crate::AccountID> {
        self.has_unauthorize().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfUnauthorize"))
        })
    }

    pub fn has_unauthorize(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfUnauthorize"))
    }

    pub fn get_authorize_credentials(&self) -> Option<crate::STArray> {
        self.has_authorize_credentials().then(|| {
            self.base
                .as_sttx()
                .get_field_array(crate::get_field_by_symbol("sfAuthorizeCredentials"))
        })
    }

    pub fn has_authorize_credentials(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAuthorizeCredentials"))
    }

    pub fn get_unauthorize_credentials(&self) -> Option<crate::STArray> {
        self.has_unauthorize_credentials().then(|| {
            self.base
                .as_sttx()
                .get_field_array(crate::get_field_by_symbol("sfUnauthorizeCredentials"))
        })
    }

    pub fn has_unauthorize_credentials(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfUnauthorizeCredentials"))
    }
}

impl Deref for DepositPreauth {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauthBuilder {
    base: crate::TransactionBuilderBase,
}

impl DepositPreauthBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(
                DepositPreauth::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        }
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != DepositPreauth::TX_TYPE {
            return Err("Invalid transaction type for DepositPreauthBuilder".to_owned());
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

    pub fn set_authorize(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfAuthorize"), value);
        self
    }

    pub fn set_unauthorize(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfUnauthorize"), value);
        self
    }

    pub fn set_authorize_credentials(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfAuthorizeCredentials"), value);
        self
    }

    pub fn set_unauthorize_credentials(mut self, value: crate::STArray) -> Self {
        self.base.object_mut().set_field_array(
            crate::get_field_by_symbol("sfUnauthorizeCredentials"),
            value,
        );
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<DepositPreauth, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(DepositPreauth::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
