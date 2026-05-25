use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerList {
    base: crate::LedgerEntryBase,
}

impl SignerList {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::SignerList;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for SignerList".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_owner(&self) -> Option<crate::AccountID> {
        self.has_owner().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_account_id(crate::get_field_by_symbol("sfOwner"))
        })
    }

    pub fn has_owner(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfOwner"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_signer_quorum(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSignerQuorum"))
    }

    pub fn get_signer_entries(&self) -> &crate::STArray {
        self.base
            .as_st_ledger_entry()
            .peek_at_pfield(crate::get_field_by_symbol("sfSignerEntries"))
            .and_then(|value| value.as_any().downcast_ref::<crate::STArray>())
            .expect("sfSignerEntries should be present on this ledger entry")
    }

    pub fn get_signer_list_id(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSignerListID"))
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

impl Deref for SignerList {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl SignerListBuilder {
    pub fn new(
        owner_node: u64,
        signer_quorum: u32,
        signer_entries: crate::STArray,
        signer_list_id: u32,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(SignerList::ENTRY_TYPE),
        };
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_signer_quorum(signer_quorum);
        builder = builder.set_signer_entries(signer_entries);
        builder = builder.set_signer_list_id(signer_list_id);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != SignerList::ENTRY_TYPE {
            return Err("Invalid ledger entry type for SignerList".to_owned());
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

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_signer_quorum(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSignerQuorum"), value);
        self
    }

    pub fn set_signer_entries(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfSignerEntries"), value);
        self
    }

    pub fn set_signer_list_id(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSignerListID"), value);
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

    pub fn build(self, index: Uint256) -> SignerList {
        SignerList::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for SignerListBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
