use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentChannelClaim {
    base: crate::TransactionBase,
}

impl PaymentChannelClaim {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(15);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for PaymentChannelClaim".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_channel(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_sttx()
            .get_field_h256(crate::get_field_by_symbol("sfChannel"))
    }

    pub fn get_amount(&self) -> Option<crate::STAmount> {
        self.has_amount().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfAmount"))
        })
    }

    pub fn has_amount(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_balance(&self) -> Option<crate::STAmount> {
        self.has_balance().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfBalance"))
        })
    }

    pub fn has_balance(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfBalance"))
    }

    pub fn get_signature(&self) -> Option<Vec<u8>> {
        self.has_signature().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfSignature"))
        })
    }

    pub fn has_signature(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfSignature"))
    }

    pub fn get_public_key(&self) -> Option<Vec<u8>> {
        self.has_public_key().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfPublicKey"))
        })
    }

    pub fn has_public_key(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfPublicKey"))
    }

    pub fn get_credential_ids(&self) -> Option<crate::STVector256> {
        self.has_credential_ids().then(|| {
            self.base
                .as_sttx()
                .get_field_v256(crate::get_field_by_symbol("sfCredentialIDs"))
        })
    }

    pub fn has_credential_ids(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCredentialIDs"))
    }
}

impl Deref for PaymentChannelClaim {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentChannelClaimBuilder {
    base: crate::TransactionBuilderBase,
}

impl PaymentChannelClaimBuilder {
    pub fn new(
        account: crate::AccountID,
        channel: basics::base_uint::Uint256,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                PaymentChannelClaim::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_channel(channel);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != PaymentChannelClaim::TX_TYPE {
            return Err("Invalid transaction type for PaymentChannelClaimBuilder".to_owned());
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

    pub fn set_channel(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfChannel"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_balance(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfBalance"), value);
        self
    }

    pub fn set_signature(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfSignature"), value.as_ref());
        self
    }

    pub fn set_public_key(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfPublicKey"), value.as_ref());
        self
    }

    pub fn set_credential_ids(mut self, value: crate::STVector256) -> Self {
        self.base
            .object_mut()
            .set_field_v256(crate::get_field_by_symbol("sfCredentialIDs"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<PaymentChannelClaim, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(
            PaymentChannelClaim::new(Arc::new(crate::STTx::from_stobject(
                self.base.into_object(),
            )))
            .expect("builder produced the matching transaction wrapper"),
        )
    }
}
