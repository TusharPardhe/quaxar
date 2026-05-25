use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTokenAcceptOffer {
    base: crate::TransactionBase,
}

impl NFTokenAcceptOffer {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(29);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for NFTokenAcceptOffer".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_nf_token_buy_offer(&self) -> Option<basics::base_uint::Uint256> {
        self.has_nf_token_buy_offer().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfNFTokenBuyOffer"))
        })
    }

    pub fn has_nf_token_buy_offer(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenBuyOffer"))
    }

    pub fn get_nf_token_sell_offer(&self) -> Option<basics::base_uint::Uint256> {
        self.has_nf_token_sell_offer().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfNFTokenSellOffer"))
        })
    }

    pub fn has_nf_token_sell_offer(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenSellOffer"))
    }

    pub fn get_nf_token_broker_fee(&self) -> Option<crate::STAmount> {
        self.has_nf_token_broker_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfNFTokenBrokerFee"))
        })
    }

    pub fn has_nf_token_broker_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenBrokerFee"))
    }
}

impl Deref for NFTokenAcceptOffer {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTokenAcceptOfferBuilder {
    base: crate::TransactionBuilderBase,
}

impl NFTokenAcceptOfferBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(
                NFTokenAcceptOffer::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        }
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != NFTokenAcceptOffer::TX_TYPE {
            return Err("Invalid transaction type for NFTokenAcceptOfferBuilder".to_owned());
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

    pub fn set_nf_token_buy_offer(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfNFTokenBuyOffer"), value);
        self
    }

    pub fn set_nf_token_sell_offer(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfNFTokenSellOffer"), value);
        self
    }

    pub fn set_nf_token_broker_fee(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfNFTokenBrokerFee"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<NFTokenAcceptOffer, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(NFTokenAcceptOffer::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
