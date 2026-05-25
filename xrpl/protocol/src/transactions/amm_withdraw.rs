use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMWithdraw {
    base: crate::TransactionBase,
}

impl AMMWithdraw {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(37);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for AMMWithdraw".to_owned());
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

    pub fn get_asset2(&self) -> crate::STIssue {
        self.base
            .as_sttx()
            .get_field_issue(crate::get_field_by_symbol("sfAsset2"))
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

    pub fn get_amount2(&self) -> Option<crate::STAmount> {
        self.has_amount2().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfAmount2"))
        })
    }

    pub fn has_amount2(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfAmount2"))
    }

    pub fn get_e_price(&self) -> Option<crate::STAmount> {
        self.has_e_price().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfEPrice"))
        })
    }

    pub fn has_e_price(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfEPrice"))
    }

    pub fn get_lp_token_in(&self) -> Option<crate::STAmount> {
        self.has_lp_token_in().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfLPTokenIn"))
        })
    }

    pub fn has_lp_token_in(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLPTokenIn"))
    }
}

impl Deref for AMMWithdraw {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMWithdrawBuilder {
    base: crate::TransactionBuilderBase,
}

impl AMMWithdrawBuilder {
    pub fn new(
        account: crate::AccountID,
        asset: crate::STIssue,
        asset2: crate::STIssue,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(AMMWithdraw::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_asset(asset);
        builder = builder.set_asset2(asset2);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != AMMWithdraw::TX_TYPE {
            return Err("Invalid transaction type for AMMWithdrawBuilder".to_owned());
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

    pub fn set_asset2(mut self, value: crate::STIssue) -> Self {
        self.base
            .object_mut()
            .set_field_issue(crate::get_field_by_symbol("sfAsset2"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_amount2(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount2"), value);
        self
    }

    pub fn set_e_price(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfEPrice"), value);
        self
    }

    pub fn set_lp_token_in(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfLPTokenIn"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<AMMWithdraw, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(AMMWithdraw::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
