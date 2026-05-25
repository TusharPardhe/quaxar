use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSet {
    base: crate::TransactionBase,
}

impl AccountSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(3);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for AccountSet".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_email_hash(&self) -> Option<basics::base_uint::Uint128> {
        self.has_email_hash().then(|| {
            self.base
                .as_sttx()
                .get_field_h128(crate::get_field_by_symbol("sfEmailHash"))
        })
    }

    pub fn has_email_hash(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfEmailHash"))
    }

    pub fn get_wallet_locator(&self) -> Option<basics::base_uint::Uint256> {
        self.has_wallet_locator().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfWalletLocator"))
        })
    }

    pub fn has_wallet_locator(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfWalletLocator"))
    }

    pub fn get_wallet_size(&self) -> Option<u32> {
        self.has_wallet_size().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfWalletSize"))
        })
    }

    pub fn has_wallet_size(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfWalletSize"))
    }

    pub fn get_message_key(&self) -> Option<Vec<u8>> {
        self.has_message_key().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfMessageKey"))
        })
    }

    pub fn has_message_key(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfMessageKey"))
    }

    pub fn get_domain(&self) -> Option<Vec<u8>> {
        self.has_domain().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfDomain"))
        })
    }

    pub fn has_domain(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDomain"))
    }

    pub fn get_transfer_rate(&self) -> Option<u32> {
        self.has_transfer_rate().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfTransferRate"))
        })
    }

    pub fn has_transfer_rate(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfTransferRate"))
    }

    pub fn get_set_flag(&self) -> Option<u32> {
        self.has_set_flag().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfSetFlag"))
        })
    }

    pub fn has_set_flag(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfSetFlag"))
    }

    pub fn get_clear_flag(&self) -> Option<u32> {
        self.has_clear_flag().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfClearFlag"))
        })
    }

    pub fn has_clear_flag(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfClearFlag"))
    }

    pub fn get_tick_size(&self) -> Option<u8> {
        self.has_tick_size().then(|| {
            self.base
                .as_sttx()
                .get_field_u8(crate::get_field_by_symbol("sfTickSize"))
        })
    }

    pub fn has_tick_size(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfTickSize"))
    }

    pub fn get_nf_token_minter(&self) -> Option<crate::AccountID> {
        self.has_nf_token_minter().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfNFTokenMinter"))
        })
    }

    pub fn has_nf_token_minter(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenMinter"))
    }
}

impl Deref for AccountSet {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSetBuilder {
    base: crate::TransactionBuilderBase,
}

impl AccountSetBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(AccountSet::TX_TYPE, account, sequence, fee),
        }
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != AccountSet::TX_TYPE {
            return Err("Invalid transaction type for AccountSetBuilder".to_owned());
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

    pub fn set_email_hash(mut self, value: basics::base_uint::Uint128) -> Self {
        self.base
            .object_mut()
            .set_field_h128(crate::get_field_by_symbol("sfEmailHash"), value);
        self
    }

    pub fn set_wallet_locator(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfWalletLocator"), value);
        self
    }

    pub fn set_wallet_size(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfWalletSize"), value);
        self
    }

    pub fn set_message_key(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfMessageKey"), value.as_ref());
        self
    }

    pub fn set_domain(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfDomain"), value.as_ref());
        self
    }

    pub fn set_transfer_rate(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfTransferRate"), value);
        self
    }

    pub fn set_set_flag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSetFlag"), value);
        self
    }

    pub fn set_clear_flag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfClearFlag"), value);
        self
    }

    pub fn set_tick_size(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfTickSize"), value);
        self
    }

    pub fn set_nf_token_minter(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfNFTokenMinter"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<AccountSet, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(AccountSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
