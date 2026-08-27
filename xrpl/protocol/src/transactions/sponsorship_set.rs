use std::{ops::Deref, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorshipSet {
    base: crate::TransactionBase,
}
impl SponsorshipSet {
    pub const TX_TYPE: crate::TxType = crate::TxType::SPONSORSHIP_SET;
    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        (tx.get_txn_type() == Self::TX_TYPE)
            .then(|| Self {
                base: crate::TransactionBase::new(tx),
            })
            .ok_or_else(|| "Invalid transaction type for SponsorshipSet".into())
    }
    pub fn get_counterparty_sponsor(&self) -> Option<crate::AccountID> {
        let f = crate::get_field_by_symbol("sfCounterpartySponsor");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_account_id(f))
    }
    pub fn get_sponsee(&self) -> Option<crate::AccountID> {
        let f = crate::get_field_by_symbol("sfSponsee");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_account_id(f))
    }
    pub fn get_fee_amount_delta(&self) -> Option<crate::STAmount> {
        let f = crate::get_field_by_symbol("sfFeeAmountDelta");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_amount(f))
    }
    pub fn get_max_fee(&self) -> Option<crate::STAmount> {
        let f = crate::get_field_by_symbol("sfMaxFee");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_amount(f))
    }
    pub fn get_remaining_owner_count_delta(&self) -> Option<i32> {
        let f = crate::get_field_by_symbol("sfRemainingOwnerCountDelta");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_i32(f))
    }
}
impl Deref for SponsorshipSet {
    type Target = crate::TransactionBase;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorshipSetBuilder {
    base: crate::TransactionBuilderBase,
}
impl SponsorshipSetBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(
                SponsorshipSet::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        }
    }
    pub fn set_counterparty_sponsor(mut self, v: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfCounterpartySponsor"), v);
        self
    }
    pub fn set_sponsee(mut self, v: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSponsee"), v);
        self
    }
    pub fn set_fee_amount_delta(mut self, v: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfFeeAmountDelta"), v);
        self
    }
    pub fn set_max_fee(mut self, v: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfMaxFee"), v);
        self
    }
    pub fn set_remaining_owner_count_delta(mut self, v: i32) -> Self {
        self.base
            .object_mut()
            .set_field_i32(crate::get_field_by_symbol("sfRemainingOwnerCountDelta"), v);
        self
    }
    pub fn set_flags(mut self, v: u32) -> Self {
        self.base.set_flags(v);
        self
    }
    pub fn set_delegate(mut self, v: crate::AccountID) -> Self {
        self.base.set_delegate(v);
        self
    }
    pub fn get_st_object(&self) -> &crate::STObject {
        self.base.object()
    }
    pub fn build(
        mut self,
        pk: &crate::PublicKey,
        sk: &crate::SecretKey,
    ) -> Result<SponsorshipSet, crate::SignError> {
        self.base.sign(pk, sk)?;
        Ok(SponsorshipSet::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .unwrap())
    }
}
