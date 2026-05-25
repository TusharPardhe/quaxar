use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSet {
    base: crate::TransactionBase,
}

impl OracleSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(51);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for OracleSet".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_oracle_document_id(&self) -> u32 {
        self.base
            .as_sttx()
            .get_field_u32(crate::get_field_by_symbol("sfOracleDocumentID"))
    }

    pub fn get_provider(&self) -> Option<Vec<u8>> {
        self.has_provider().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfProvider"))
        })
    }

    pub fn has_provider(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfProvider"))
    }

    pub fn get_uri(&self) -> Option<Vec<u8>> {
        self.has_uri().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfURI"))
        })
    }

    pub fn has_uri(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfURI"))
    }

    pub fn get_asset_class(&self) -> Option<Vec<u8>> {
        self.has_asset_class().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfAssetClass"))
        })
    }

    pub fn has_asset_class(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAssetClass"))
    }

    pub fn get_last_update_time(&self) -> u32 {
        self.base
            .as_sttx()
            .get_field_u32(crate::get_field_by_symbol("sfLastUpdateTime"))
    }

    pub fn get_price_data_series(&self) -> crate::STArray {
        self.base
            .as_sttx()
            .get_field_array(crate::get_field_by_symbol("sfPriceDataSeries"))
    }
}

impl Deref for OracleSet {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetBuilder {
    base: crate::TransactionBuilderBase,
}

impl OracleSetBuilder {
    pub fn new(
        account: crate::AccountID,
        oracle_document_id: u32,
        last_update_time: u32,
        price_data_series: crate::STArray,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(OracleSet::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_oracle_document_id(oracle_document_id);
        builder = builder.set_last_update_time(last_update_time);
        builder = builder.set_price_data_series(price_data_series);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != OracleSet::TX_TYPE {
            return Err("Invalid transaction type for OracleSetBuilder".to_owned());
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

    pub fn set_oracle_document_id(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOracleDocumentID"), value);
        self
    }

    pub fn set_provider(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfProvider"), value.as_ref());
        self
    }

    pub fn set_uri(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfURI"), value.as_ref());
        self
    }

    pub fn set_asset_class(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfAssetClass"), value.as_ref());
        self
    }

    pub fn set_last_update_time(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLastUpdateTime"), value);
        self
    }

    pub fn set_price_data_series(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfPriceDataSeries"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<OracleSet, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(OracleSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
