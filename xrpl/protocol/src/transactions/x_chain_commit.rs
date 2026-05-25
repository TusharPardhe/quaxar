use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCommit {
    base: crate::TransactionBase,
}

impl XChainCommit {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(42);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for XChainCommit".to_owned());
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

    pub fn get_x_chain_claim_id(&self) -> u64 {
        self.base
            .as_sttx()
            .get_field_u64(crate::get_field_by_symbol("sfXChainClaimID"))
    }

    pub fn get_amount(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_other_chain_destination(&self) -> Option<crate::AccountID> {
        self.has_other_chain_destination().then(|| {
            self.base
                .as_sttx()
                .get_account_id(crate::get_field_by_symbol("sfOtherChainDestination"))
        })
    }

    pub fn has_other_chain_destination(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfOtherChainDestination"))
    }
}

impl Deref for XChainCommit {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCommitBuilder {
    base: crate::TransactionBuilderBase,
}

impl XChainCommitBuilder {
    pub fn new(
        account: crate::AccountID,
        x_chain_bridge: crate::STXChainBridge,
        x_chain_claim_id: u64,
        amount: crate::STAmount,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(XChainCommit::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_x_chain_bridge(x_chain_bridge);
        builder = builder.set_x_chain_claim_id(x_chain_claim_id);
        builder = builder.set_amount(amount);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != XChainCommit::TX_TYPE {
            return Err("Invalid transaction type for XChainCommitBuilder".to_owned());
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

    pub fn set_x_chain_claim_id(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfXChainClaimID"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_other_chain_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOtherChainDestination"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<XChainCommit, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(XChainCommit::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
