use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateBridge {
    base: crate::TransactionBase,
}

impl XChainCreateBridge {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(48);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for XChainCreateBridge".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_x_chain_bridge(&self) -> crate::STXChainBridge {
        self.base
            .as_sttx()
            .get_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"))
    }

    pub fn get_signature_reward(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfSignatureReward"))
    }

    pub fn get_min_account_create_amount(&self) -> Option<crate::STAmount> {
        self.has_min_account_create_amount().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfMinAccountCreateAmount"))
        })
    }

    pub fn has_min_account_create_amount(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfMinAccountCreateAmount"))
    }
}

impl Deref for XChainCreateBridge {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateBridgeBuilder {
    base: crate::TransactionBuilderBase,
}

impl XChainCreateBridgeBuilder {
    pub fn new(
        account: crate::AccountID,
        x_chain_bridge: crate::STXChainBridge,
        signature_reward: crate::STAmount,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                XChainCreateBridge::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_x_chain_bridge(x_chain_bridge);
        builder = builder.set_signature_reward(signature_reward);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != XChainCreateBridge::TX_TYPE {
            return Err("Invalid transaction type for XChainCreateBridgeBuilder".to_owned());
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

    pub fn set_x_chain_bridge(mut self, value: crate::STXChainBridge) -> Self {
        self.base
            .object_mut()
            .set_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"), value);
        self
    }

    pub fn set_signature_reward(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfSignatureReward"), value);
        self
    }

    pub fn set_min_account_create_amount(mut self, value: crate::STAmount) -> Self {
        self.base.object_mut().set_field_amount(
            crate::get_field_by_symbol("sfMinAccountCreateAmount"),
            value,
        );
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<XChainCreateBridge, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(XChainCreateBridge::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
