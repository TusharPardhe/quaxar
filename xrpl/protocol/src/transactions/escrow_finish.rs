use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscrowFinish {
    base: crate::TransactionBase,
}

impl EscrowFinish {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(2);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for EscrowFinish".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_owner(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfOwner"))
    }

    pub fn get_offer_sequence(&self) -> u32 {
        self.base
            .as_sttx()
            .get_field_u32(crate::get_field_by_symbol("sfOfferSequence"))
    }

    pub fn get_fulfillment(&self) -> Option<Vec<u8>> {
        self.has_fulfillment().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfFulfillment"))
        })
    }

    pub fn has_fulfillment(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfFulfillment"))
    }

    pub fn get_condition(&self) -> Option<Vec<u8>> {
        self.has_condition().then(|| {
            self.base
                .as_sttx()
                .get_field_vl(crate::get_field_by_symbol("sfCondition"))
        })
    }

    pub fn has_condition(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCondition"))
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

impl Deref for EscrowFinish {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscrowFinishBuilder {
    base: crate::TransactionBuilderBase,
}

impl EscrowFinishBuilder {
    pub fn new(
        account: crate::AccountID,
        owner: crate::AccountID,
        offer_sequence: u32,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(EscrowFinish::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_owner(owner);
        builder = builder.set_offer_sequence(offer_sequence);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != EscrowFinish::TX_TYPE {
            return Err("Invalid transaction type for EscrowFinishBuilder".to_owned());
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

    pub fn set_owner(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOwner"), value);
        self
    }

    pub fn set_offer_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOfferSequence"), value);
        self
    }

    pub fn set_fulfillment(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfFulfillment"), value.as_ref());
        self
    }

    pub fn set_condition(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfCondition"), value.as_ref());
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
    ) -> Result<EscrowFinish, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(EscrowFinish::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
