use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegate {
    base: crate::LedgerEntryBase,
}

impl Delegate {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Delegate;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Delegate".to_owned());
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

    pub fn get_authorize(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfAuthorize"))
    }

    pub fn get_permissions(&self) -> &crate::STArray {
        self.base
            .as_st_ledger_entry()
            .peek_at_pfield(crate::get_field_by_symbol("sfPermissions"))
            .and_then(|value| value.as_any().downcast_ref::<crate::STArray>())
            .expect("sfPermissions should be present on this ledger entry")
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

impl Deref for Delegate {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl DelegateBuilder {
    pub fn new(
        account: crate::AccountID,
        authorize: crate::AccountID,
        permissions: crate::STArray,
        owner_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Delegate::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_authorize(authorize);
        builder = builder.set_permissions(permissions);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Delegate::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Delegate".to_owned());
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

    pub fn set_authorize(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfAuthorize"), value);
        self
    }

    pub fn set_permissions(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfPermissions"), value);
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

    pub fn build(self, index: Uint256) -> Delegate {
        Delegate::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for DelegateBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
