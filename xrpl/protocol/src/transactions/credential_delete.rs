use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialDelete {
    base: crate::TransactionBase,
}

impl CredentialDelete {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(60);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for CredentialDelete".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_subject(&self) -> Option<crate::AccountID> {
        self.has_subject().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfSubject"))
        })
    }

    pub fn has_subject(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfSubject"))
    }

    pub fn get_issuer(&self) -> Option<crate::AccountID> {
        self.has_issuer().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfIssuer"))
        })
    }

    pub fn has_issuer(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfIssuer"))
    }

    pub fn get_credential_type(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(crate::get_field_by_symbol("sfCredentialType"))
    }
}

impl Deref for CredentialDelete {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialDeleteBuilder {
    base: crate::TransactionBuilderBase,
}

impl CredentialDeleteBuilder {
    pub fn new(
        account: crate::AccountID,
        credential_type: impl AsRef<[u8]>,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                CredentialDelete::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_credential_type(credential_type);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != CredentialDelete::TX_TYPE {
            return Err("Invalid transaction type for CredentialDeleteBuilder".to_owned());
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

    pub fn set_subject(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSubject"), value);
        self
    }

    pub fn set_issuer(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfIssuer"), value);
        self
    }

    pub fn set_credential_type(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfCredentialType"),
            value.as_ref(),
        );
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<CredentialDelete, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(CredentialDelete::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
