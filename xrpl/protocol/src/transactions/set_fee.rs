use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFee {
    base: crate::TransactionBase,
}

impl SetFee {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(101);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for SetFee".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_ledger_sequence(&self) -> Option<u32> {
        self.has_ledger_sequence().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfLedgerSequence"))
        })
    }

    pub fn has_ledger_sequence(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfLedgerSequence"))
    }

    pub fn get_base_fee(&self) -> Option<u64> {
        self.has_base_fee().then(|| {
            self.base
                .as_sttx()
                .get_field_u64(crate::get_field_by_symbol("sfBaseFee"))
        })
    }

    pub fn has_base_fee(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfBaseFee"))
    }

    pub fn get_reference_fee_units(&self) -> Option<u32> {
        self.has_reference_fee_units().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfReferenceFeeUnits"))
        })
    }

    pub fn has_reference_fee_units(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfReferenceFeeUnits"))
    }

    pub fn get_reserve_base(&self) -> Option<u32> {
        self.has_reserve_base().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfReserveBase"))
        })
    }

    pub fn has_reserve_base(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfReserveBase"))
    }

    pub fn get_reserve_increment(&self) -> Option<u32> {
        self.has_reserve_increment().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfReserveIncrement"))
        })
    }

    pub fn has_reserve_increment(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfReserveIncrement"))
    }

    pub fn get_base_fee_drops(&self) -> Option<crate::STAmount> {
        self.has_base_fee_drops().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfBaseFeeDrops"))
        })
    }

    pub fn has_base_fee_drops(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfBaseFeeDrops"))
    }

    pub fn get_reserve_base_drops(&self) -> Option<crate::STAmount> {
        self.has_reserve_base_drops().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfReserveBaseDrops"))
        })
    }

    pub fn has_reserve_base_drops(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfReserveBaseDrops"))
    }

    pub fn get_reserve_increment_drops(&self) -> Option<crate::STAmount> {
        self.has_reserve_increment_drops().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfReserveIncrementDrops"))
        })
    }

    pub fn has_reserve_increment_drops(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfReserveIncrementDrops"))
    }
}

impl Deref for SetFee {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFeeBuilder {
    base: crate::TransactionBuilderBase,
}

impl SetFeeBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(SetFee::TX_TYPE, account, sequence, fee),
        }
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != SetFee::TX_TYPE {
            return Err("Invalid transaction type for SetFeeBuilder".to_owned());
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

    pub fn set_ledger_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfLedgerSequence"), value);
        self
    }

    pub fn set_base_fee(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfBaseFee"), value);
        self
    }

    pub fn set_reference_fee_units(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReferenceFeeUnits"), value);
        self
    }

    pub fn set_reserve_base(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReserveBase"), value);
        self
    }

    pub fn set_reserve_increment(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfReserveIncrement"), value);
        self
    }

    pub fn set_base_fee_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfBaseFeeDrops"), value);
        self
    }

    pub fn set_reserve_base_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfReserveBaseDrops"), value);
        self
    }

    pub fn set_reserve_increment_drops(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfReserveIncrementDrops"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<SetFee, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(SetFee::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
