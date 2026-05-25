use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuance {
    base: crate::LedgerEntryBase,
}

impl MPTokenIssuance {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::MPTokenIssuance;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for MPTokenIssuance".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_issuer(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfIssuer"))
    }

    pub fn get_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSequence"))
    }

    pub fn get_transfer_fee(&self) -> Option<u16> {
        self.has_transfer_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u16(crate::get_field_by_symbol("sfTransferFee"))
        })
    }

    pub fn has_transfer_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTransferFee"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_asset_scale(&self) -> Option<u8> {
        self.has_asset_scale().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u8(crate::get_field_by_symbol("sfAssetScale"))
        })
    }

    pub fn has_asset_scale(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAssetScale"))
    }

    pub fn get_maximum_amount(&self) -> Option<u64> {
        self.has_maximum_amount().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfMaximumAmount"))
        })
    }

    pub fn has_maximum_amount(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMaximumAmount"))
    }

    pub fn get_outstanding_amount(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOutstandingAmount"))
    }

    pub fn get_locked_amount(&self) -> Option<u64> {
        self.has_locked_amount().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfLockedAmount"))
        })
    }

    pub fn has_locked_amount(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLockedAmount"))
    }

    pub fn get_mp_token_metadata(&self) -> Option<Vec<u8>> {
        self.has_mp_token_metadata().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfMPTokenMetadata"))
        })
    }

    pub fn has_mp_token_metadata(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMPTokenMetadata"))
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

    pub fn get_domain_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_domain_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfDomainID"))
        })
    }

    pub fn has_domain_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDomainID"))
    }

    pub fn get_mutable_flags(&self) -> Option<u32> {
        self.has_mutable_flags().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfMutableFlags"))
        })
    }

    pub fn has_mutable_flags(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMutableFlags"))
    }
}

impl Deref for MPTokenIssuance {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl MPTokenIssuanceBuilder {
    pub fn new(
        issuer: crate::AccountID,
        sequence: u32,
        owner_node: u64,
        outstanding_amount: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(MPTokenIssuance::ENTRY_TYPE),
        };
        builder = builder.set_issuer(issuer);
        builder = builder.set_sequence(sequence);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_outstanding_amount(outstanding_amount);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != MPTokenIssuance::ENTRY_TYPE {
            return Err("Invalid ledger entry type for MPTokenIssuance".to_owned());
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

    pub fn set_issuer(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfIssuer"), value);
        self
    }

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSequence"), value);
        self
    }

    pub fn set_transfer_fee(mut self, value: u16) -> Self {
        self.base
            .object_mut()
            .set_field_u16(crate::get_field_by_symbol("sfTransferFee"), value);
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_asset_scale(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfAssetScale"), value);
        self
    }

    pub fn set_maximum_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfMaximumAmount"), value);
        self
    }

    pub fn set_outstanding_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOutstandingAmount"), value);
        self
    }

    pub fn set_locked_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfLockedAmount"), value);
        self
    }

    pub fn set_mp_token_metadata(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfMPTokenMetadata"),
            value.as_ref(),
        );
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

    pub fn set_domain_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfDomainID"), value);
        self
    }

    pub fn set_mutable_flags(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfMutableFlags"), value);
        self
    }

    pub fn build(self, index: Uint256) -> MPTokenIssuance {
        MPTokenIssuance::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for MPTokenIssuanceBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
