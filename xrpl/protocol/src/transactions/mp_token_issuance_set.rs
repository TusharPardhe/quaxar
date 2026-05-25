use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceSet {
    base: crate::TransactionBase,
}

impl MPTokenIssuanceSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(56);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for MPTokenIssuanceSet".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_mp_token_issuance_id(&self) -> basics::base_uint::Uint192 {
        self.base
            .as_sttx()
            .get_field_h192(crate::get_field_by_symbol("sfMPTokenIssuanceID"))
    }

    pub fn get_holder(&self) -> Option<crate::AccountID> {
        self.has_holder().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfHolder"))
        })
    }

    pub fn has_holder(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfHolder"))
    }

    pub fn get_domain_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_domain_id().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfDomainID"))
        })
    }

    pub fn has_domain_id(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDomainID"))
    }

    pub fn get_mp_token_metadata(&self) -> Option<Vec<u8>> {
        self.has_mp_token_metadata().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfMPTokenMetadata"))
        })
    }

    pub fn has_mp_token_metadata(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfMPTokenMetadata"))
    }

    pub fn get_transfer_fee(&self) -> Option<u16> {
        self.has_transfer_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_u16(crate::get_field_by_symbol("sfTransferFee"))
        })
    }

    pub fn has_transfer_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfTransferFee"))
    }

    pub fn get_mutable_flags(&self) -> Option<u32> {
        self.has_mutable_flags().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfMutableFlags"))
        })
    }

    pub fn has_mutable_flags(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfMutableFlags"))
    }
}

impl Deref for MPTokenIssuanceSet {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceSetBuilder {
    base: crate::TransactionBuilderBase,
}

impl MPTokenIssuanceSetBuilder {
    pub fn new(
        account: crate::AccountID,
        mp_token_issuance_id: basics::base_uint::Uint192,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                MPTokenIssuanceSet::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_mp_token_issuance_id(mp_token_issuance_id);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != MPTokenIssuanceSet::TX_TYPE {
            return Err("Invalid transaction type for MPTokenIssuanceSetBuilder".to_owned());
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

    pub fn set_mp_token_issuance_id(mut self, value: basics::base_uint::Uint192) -> Self {
        self.base
            .object_mut()
            .set_field_h192(crate::get_field_by_symbol("sfMPTokenIssuanceID"), value);
        self
    }

    pub fn set_holder(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfHolder"), value);
        self
    }

    pub fn set_domain_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfDomainID"), value);
        self
    }

    pub fn set_mp_token_metadata(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfMPTokenMetadata"),
            value.as_ref(),
        );
        self
    }

    pub fn set_transfer_fee(mut self, value: u16) -> Self {
        self.base
            .object_mut()
            .set_field_u16(crate::get_field_by_symbol("sfTransferFee"), value);
        self
    }

    pub fn set_mutable_flags(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfMutableFlags"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<MPTokenIssuanceSet, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(MPTokenIssuanceSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
