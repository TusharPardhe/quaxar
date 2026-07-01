use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

/// Ledger object flags for ltSPONSORSHIP.
pub const LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE: u32 = 0x0001_0000;
pub const LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE: u32 = 0x0002_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sponsorship {
    base: crate::LedgerEntryBase,
}

impl Sponsorship {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Sponsorship;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Sponsorship".to_owned());
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

    pub fn get_sponsee(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfSponsee"))
    }

    pub fn get_remaining_owner_count(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfRemainingOwnerCount"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_sponsee_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfSponseeNode"))
    }

    pub fn get_previous_txn_id(&self) -> Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
    }

    pub fn get_flags(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfFlags"))
    }

    pub fn is_require_sign_for_fee(&self) -> bool {
        self.get_flags() & LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE != 0
    }

    pub fn is_require_sign_for_reserve(&self) -> bool {
        self.get_flags() & LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE != 0
    }
}

impl Deref for Sponsorship {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorshipBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl SponsorshipBuilder {
    pub fn new(
        owner: crate::AccountID,
        sponsee: crate::AccountID,
        owner_node: u64,
        sponsee_node: u64,
        previous_txn_id: Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Sponsorship::ENTRY_TYPE),
        };
        builder = builder.set_owner(owner);
        builder = builder.set_sponsee(sponsee);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_sponsee_node(sponsee_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Sponsorship::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Sponsorship".to_owned());
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

    pub fn set_sponsee(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSponsee"), value);
        self
    }

    pub fn set_remaining_owner_count(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfRemainingOwnerCount"), value);
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_sponsee_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfSponseeNode"), value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: Uint256) -> Self {
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

    pub fn build(self, index: Uint256) -> Sponsorship {
        Sponsorship::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for SponsorshipBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
