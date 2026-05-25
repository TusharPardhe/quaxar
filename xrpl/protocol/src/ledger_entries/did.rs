use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DID {
    base: crate::LedgerEntryBase,
}

impl DID {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::DID;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for DID".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_account(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfAccount"))
    }

    pub fn get_did_document(&self) -> Option<Vec<u8>> {
        self.has_did_document().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfDIDDocument"))
        })
    }

    pub fn has_did_document(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDIDDocument"))
    }

    pub fn get_uri(&self) -> Option<Vec<u8>> {
        self.has_uri().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfURI"))
        })
    }

    pub fn has_uri(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfURI"))
    }

    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.has_data().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfData"))
        })
    }

    pub fn has_data(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfData"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
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

impl Deref for DID {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DIDBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl DIDBuilder {
    pub fn new(
        account: crate::AccountID,
        owner_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(DID::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != DID::ENTRY_TYPE {
            return Err("Invalid ledger entry type for DID".to_owned());
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

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfAccount"), value);
        self
    }

    pub fn set_did_document(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfDIDDocument"), value.as_ref());
        self
    }

    pub fn set_uri(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfURI"), value.as_ref());
        self
    }

    pub fn set_data(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfData"), value.as_ref());
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
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

    pub fn build(self, index: Uint256) -> DID {
        DID::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for DIDBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
