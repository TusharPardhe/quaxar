use std::{ops::Deref, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorshipTransfer {
    base: crate::TransactionBase,
}
impl SponsorshipTransfer {
    pub const TX_TYPE: crate::TxType = crate::TxType::SPONSORSHIP_TRANSFER;
    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        (tx.get_txn_type() == Self::TX_TYPE)
            .then(|| Self {
                base: crate::TransactionBase::new(tx),
            })
            .ok_or_else(|| "Invalid transaction type for SponsorshipTransfer".into())
    }
    pub fn get_object_id(&self) -> Option<basics::base_uint::Uint256> {
        let f = crate::get_field_by_symbol("sfObjectID");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_h256(f))
    }
    pub fn get_sponsee(&self) -> Option<crate::AccountID> {
        let f = crate::get_field_by_symbol("sfSponsee");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_account_id(f))
    }
}
impl Deref for SponsorshipTransfer {
    type Target = crate::TransactionBase;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorshipTransferBuilder {
    base: crate::TransactionBuilderBase,
}
impl SponsorshipTransferBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self {
            base: crate::TransactionBuilderBase::new(
                SponsorshipTransfer::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        }
    }
    pub fn set_object_id(mut self, v: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfObjectID"), v);
        self
    }
    pub fn set_sponsee(mut self, v: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSponsee"), v);
        self
    }
    pub fn set_sponsor(mut self, v: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSponsor"), v);
        self
    }
    pub fn set_sponsor_flags(mut self, v: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSponsorFlags"), v);
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
    ) -> Result<SponsorshipTransfer, crate::SignError> {
        self.base.sign(pk, sk)?;
        Ok(
            SponsorshipTransfer::new(Arc::new(crate::STTx::from_stobject(
                self.base.into_object(),
            )))
            .unwrap(),
        )
    }
}
