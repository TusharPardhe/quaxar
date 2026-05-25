use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTokenOffer {
    base: crate::LedgerEntryBase,
}

impl NFTokenOffer {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::NFTokenOffer;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for NFTokenOffer".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_owner(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfOwner"))
    }

    pub fn get_nf_token_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfNFTokenID"))
    }

    pub fn get_amount(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_nf_token_offer_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfNFTokenOfferNode"))
    }

    pub fn get_destination(&self) -> Option<crate::AccountID> {
        self.has_destination().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_account_id(crate::get_field_by_symbol("sfDestination"))
        })
    }

    pub fn has_destination(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDestination"))
    }

    pub fn get_expiration(&self) -> Option<u32> {
        self.has_expiration().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfExpiration"))
        })
    }

    pub fn has_expiration(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfExpiration"))
    }

    pub fn get_previous_txn_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
    }
}

impl Deref for NFTokenOffer {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTokenOfferBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl NFTokenOfferBuilder {
    pub fn new(
        owner: crate::AccountID,
        nf_token_id: basics::base_uint::Uint256,
        amount: crate::STAmount,
        owner_node: u64,
        nf_token_offer_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(NFTokenOffer::ENTRY_TYPE),
        };
        builder = builder.set_owner(owner);
        builder = builder.set_nf_token_id(nf_token_id);
        builder = builder.set_amount(amount);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_nf_token_offer_node(nf_token_offer_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != NFTokenOffer::ENTRY_TYPE {
            return Err("Invalid ledger entry type for NFTokenOffer".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBuilderBase::from_sle(sle),
        })
    }

    pub fn set_ledger_index(mut self, value: Uint256) -> Self {
        self.base.set_ledger_index(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_owner(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOwner"), value);
        self
    }

    pub fn set_nf_token_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfNFTokenID"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_nf_token_offer_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfNFTokenOfferNode"), value);
        self
    }

    pub fn set_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfDestination"), value);
        self
    }

    pub fn set_expiration(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfExpiration"), value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"), value);
        self
    }

    pub fn set_previous_txn_lgr_seq(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"), value);
        self
    }

    pub fn build(self, index: Uint256) -> NFTokenOffer {
        NFTokenOffer::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for NFTokenOfferBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
