use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreate {
    base: crate::TransactionBase,
}

impl VaultCreate {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(65);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for VaultCreate".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_asset(&self) -> crate::STIssue {
        self.base
            .as_sttx()
            .get_field_issue(crate::get_field_by_symbol("sfAsset"))
    }

    pub fn get_assets_maximum(&self) -> Option<crate::STNumber> {
        self.has_assets_maximum().then(|| {
            self.base
                .as_sttx()
                .get_field_number(crate::get_field_by_symbol("sfAssetsMaximum"))
        })
    }

    pub fn has_assets_maximum(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAssetsMaximum"))
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

    pub fn get_withdrawal_policy(&self) -> Option<u8> {
        self.has_withdrawal_policy().then(|| {
            self.base
                .as_sttx()
                .get_field_u8(crate::get_field_by_symbol("sfWithdrawalPolicy"))
        })
    }

    pub fn has_withdrawal_policy(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfWithdrawalPolicy"))
    }

    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.has_data().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfData"))
        })
    }

    pub fn has_data(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfData"))
    }

    pub fn get_scale(&self) -> Option<u8> {
        self.has_scale().then(|| {
            self.base
                .as_sttx()
                .get_field_u8(crate::get_field_by_symbol("sfScale"))
        })
    }

    pub fn has_scale(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfScale"))
    }
}

impl Deref for VaultCreate {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateBuilder {
    base: crate::TransactionBuilderBase,
}

impl VaultCreateBuilder {
    pub fn new(
        account: crate::AccountID,
        asset: crate::STIssue,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(VaultCreate::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_asset(asset);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != VaultCreate::TX_TYPE {
            return Err("Invalid transaction type for VaultCreateBuilder".to_owned());
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

    pub fn set_asset(mut self, value: crate::STIssue) -> Self {
        self.base
            .object_mut()
            .set_field_issue(crate::get_field_by_symbol("sfAsset"), value);
        self
    }

    pub fn set_assets_maximum(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfAssetsMaximum"), value);
        self
    }

    pub fn set_mp_token_metadata(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfMPTokenMetadata"),
            value.as_ref(),
        );
        self
    }

    pub fn set_domain_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfDomainID"), value);
        self
    }

    pub fn set_withdrawal_policy(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfWithdrawalPolicy"), value);
        self
    }

    pub fn set_data(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfData"), value.as_ref());
        self
    }

    pub fn set_scale(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfScale"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<VaultCreate, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(VaultCreate::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
