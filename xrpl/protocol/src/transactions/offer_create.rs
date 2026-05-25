use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCreate {
    base: crate::TransactionBase,
}

impl OfferCreate {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(7);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for OfferCreate".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_taker_pays(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfTakerPays"))
    }

    pub fn get_taker_gets(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfTakerGets"))
    }

    pub fn get_expiration(&self) -> Option<u32> {
        self.has_expiration().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfExpiration"))
        })
    }

    pub fn has_expiration(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfExpiration"))
    }

    pub fn get_offer_sequence(&self) -> Option<u32> {
        self.has_offer_sequence().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfOfferSequence"))
        })
    }

    pub fn has_offer_sequence(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfOfferSequence"))
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
}

impl Deref for OfferCreate {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCreateBuilder {
    base: crate::TransactionBuilderBase,
}

impl OfferCreateBuilder {
    pub fn new(
        account: crate::AccountID,
        taker_pays: crate::STAmount,
        taker_gets: crate::STAmount,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(OfferCreate::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_taker_pays(taker_pays);
        builder = builder.set_taker_gets(taker_gets);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != OfferCreate::TX_TYPE {
            return Err("Invalid transaction type for OfferCreateBuilder".to_owned());
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

    pub fn set_taker_pays(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfTakerPays"), value);
        self
    }

    pub fn set_taker_gets(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfTakerGets"), value);
        self
    }

    pub fn set_expiration(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfExpiration"), value);
        self
    }

    pub fn set_offer_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOfferSequence"), value);
        self
    }

    pub fn set_domain_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfDomainID"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<OfferCreate, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(OfferCreate::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
