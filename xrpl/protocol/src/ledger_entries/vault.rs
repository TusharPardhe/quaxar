use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    base: crate::LedgerEntryBase,
}

impl Vault {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Vault;

    #[allow(clippy::too_many_arguments)]
    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Vault".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
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

    pub fn get_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSequence"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
    }

    pub fn get_owner(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfOwner"))
    }

    pub fn get_account(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfAccount"))
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

    pub fn get_asset(&self) -> crate::STIssue {
        self.base
            .as_st_ledger_entry()
            .get_field_issue(crate::get_field_by_symbol("sfAsset"))
    }

    pub fn get_assets_total(&self) -> Option<crate::STNumber> {
        self.has_assets_total().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfAssetsTotal"))
        })
    }

    pub fn has_assets_total(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAssetsTotal"))
    }

    pub fn get_assets_available(&self) -> Option<crate::STNumber> {
        self.has_assets_available().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfAssetsAvailable"))
        })
    }

    pub fn has_assets_available(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAssetsAvailable"))
    }

    pub fn get_assets_maximum(&self) -> Option<crate::STNumber> {
        self.has_assets_maximum().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfAssetsMaximum"))
        })
    }

    pub fn has_assets_maximum(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAssetsMaximum"))
    }

    pub fn get_loss_unrealized(&self) -> Option<crate::STNumber> {
        self.has_loss_unrealized().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_number(crate::get_field_by_symbol("sfLossUnrealized"))
        })
    }

    pub fn has_loss_unrealized(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLossUnrealized"))
    }

    pub fn get_share_mptid(&self) -> basics::base_uint::Uint192 {
        self.base
            .as_st_ledger_entry()
            .get_field_h192(crate::get_field_by_symbol("sfShareMPTID"))
    }

    pub fn get_withdrawal_policy(&self) -> u8 {
        self.base
            .as_st_ledger_entry()
            .get_field_u8(crate::get_field_by_symbol("sfWithdrawalPolicy"))
    }

    pub fn get_scale(&self) -> Option<u8> {
        self.has_scale().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u8(crate::get_field_by_symbol("sfScale"))
        })
    }

    pub fn has_scale(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfScale"))
    }
}

impl Deref for Vault {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl VaultBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
        sequence: u32,
        owner_node: u64,
        owner: crate::AccountID,
        account: crate::AccountID,
        asset: crate::STIssue,
        share_mptid: basics::base_uint::Uint192,
        withdrawal_policy: u8,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Vault::ENTRY_TYPE),
        };
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder = builder.set_sequence(sequence);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_owner(owner);
        builder = builder.set_account(account);
        builder = builder.set_asset(asset);
        builder = builder.set_share_mptid(share_mptid);
        builder = builder.set_withdrawal_policy(withdrawal_policy);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Vault::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Vault".to_owned());
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

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSequence"), value);
        self
    }

    pub fn set_owner_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfOwnerNode"), value);
        self
    }

    pub fn set_owner(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOwner"), value);
        self
    }

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfAccount"), value);
        self
    }

    pub fn set_data(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfData"), value.as_ref());
        self
    }

    pub fn set_asset(mut self, value: crate::STIssue) -> Self {
        self.base
            .object_mut()
            .set_field_issue(crate::get_field_by_symbol("sfAsset"), value);
        self
    }

    pub fn set_assets_total(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfAssetsTotal"), value);
        self
    }

    pub fn set_assets_available(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfAssetsAvailable"), value);
        self
    }

    pub fn set_assets_maximum(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfAssetsMaximum"), value);
        self
    }

    pub fn set_loss_unrealized(mut self, value: crate::STNumber) -> Self {
        self.base
            .object_mut()
            .set_field_number(crate::get_field_by_symbol("sfLossUnrealized"), value);
        self
    }

    pub fn set_share_mptid(mut self, value: basics::base_uint::Uint192) -> Self {
        self.base
            .object_mut()
            .set_field_h192(crate::get_field_by_symbol("sfShareMPTID"), value);
        self
    }

    pub fn set_withdrawal_policy(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfWithdrawalPolicy"), value);
        self
    }

    pub fn set_scale(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfScale"), value);
        self
    }

    pub fn build(self, index: Uint256) -> Vault {
        Vault::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for VaultBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
