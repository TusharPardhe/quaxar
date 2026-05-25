use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryNode {
    base: crate::LedgerEntryBase,
}

impl DirectoryNode {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::DirectoryNode;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for DirectoryNode".to_owned());
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

    pub fn get_taker_pays_currency(&self) -> Option<basics::base_uint::Uint160> {
        self.has_taker_pays_currency().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h160(crate::get_field_by_symbol("sfTakerPaysCurrency"))
        })
    }

    pub fn has_taker_pays_currency(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTakerPaysCurrency"))
    }

    pub fn get_taker_pays_issuer(&self) -> Option<basics::base_uint::Uint160> {
        self.has_taker_pays_issuer().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h160(crate::get_field_by_symbol("sfTakerPaysIssuer"))
        })
    }

    pub fn has_taker_pays_issuer(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTakerPaysIssuer"))
    }

    pub fn get_taker_gets_currency(&self) -> Option<basics::base_uint::Uint160> {
        self.has_taker_gets_currency().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h160(crate::get_field_by_symbol("sfTakerGetsCurrency"))
        })
    }

    pub fn has_taker_gets_currency(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTakerGetsCurrency"))
    }

    pub fn get_taker_gets_issuer(&self) -> Option<basics::base_uint::Uint160> {
        self.has_taker_gets_issuer().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h160(crate::get_field_by_symbol("sfTakerGetsIssuer"))
        })
    }

    pub fn has_taker_gets_issuer(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTakerGetsIssuer"))
    }

    pub fn get_exchange_rate(&self) -> Option<u64> {
        self.has_exchange_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfExchangeRate"))
        })
    }

    pub fn has_exchange_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfExchangeRate"))
    }

    pub fn get_indexes(&self) -> crate::STVector256 {
        self.base
            .as_st_ledger_entry()
            .get_field_v256(crate::get_field_by_symbol("sfIndexes"))
    }

    pub fn get_root_index(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfRootIndex"))
    }

    pub fn get_index_next(&self) -> Option<u64> {
        self.has_index_next().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfIndexNext"))
        })
    }

    pub fn has_index_next(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfIndexNext"))
    }

    pub fn get_index_previous(&self) -> Option<u64> {
        self.has_index_previous().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfIndexPrevious"))
        })
    }

    pub fn has_index_previous(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfIndexPrevious"))
    }

    pub fn get_nf_token_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_nf_token_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfNFTokenID"))
        })
    }

    pub fn has_nf_token_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenID"))
    }

    pub fn get_previous_txn_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_previous_txn_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
        })
    }

    pub fn has_previous_txn_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> Option<u32> {
        self.has_previous_txn_lgr_seq().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
        })
    }

    pub fn has_previous_txn_lgr_seq(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
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
}

impl Deref for DirectoryNode {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryNodeBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl DirectoryNodeBuilder {
    pub fn new(indexes: crate::STVector256, root_index: basics::base_uint::Uint256) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(DirectoryNode::ENTRY_TYPE),
        };
        builder = builder.set_indexes(indexes);
        builder = builder.set_root_index(root_index);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != DirectoryNode::ENTRY_TYPE {
            return Err("Invalid ledger entry type for DirectoryNode".to_owned());
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

    pub fn set_taker_pays_currency(mut self, value: basics::base_uint::Uint160) -> Self {
        self.base
            .object_mut()
            .set_field_h160(crate::get_field_by_symbol("sfTakerPaysCurrency"), value);
        self
    }

    pub fn set_taker_pays_issuer(mut self, value: basics::base_uint::Uint160) -> Self {
        self.base
            .object_mut()
            .set_field_h160(crate::get_field_by_symbol("sfTakerPaysIssuer"), value);
        self
    }

    pub fn set_taker_gets_currency(mut self, value: basics::base_uint::Uint160) -> Self {
        self.base
            .object_mut()
            .set_field_h160(crate::get_field_by_symbol("sfTakerGetsCurrency"), value);
        self
    }

    pub fn set_taker_gets_issuer(mut self, value: basics::base_uint::Uint160) -> Self {
        self.base
            .object_mut()
            .set_field_h160(crate::get_field_by_symbol("sfTakerGetsIssuer"), value);
        self
    }

    pub fn set_exchange_rate(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfExchangeRate"), value);
        self
    }

    pub fn set_indexes(mut self, value: crate::STVector256) -> Self {
        self.base
            .object_mut()
            .set_field_v256(crate::get_field_by_symbol("sfIndexes"), value);
        self
    }

    pub fn set_root_index(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfRootIndex"), value);
        self
    }

    pub fn set_index_next(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfIndexNext"), value);
        self
    }

    pub fn set_index_previous(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfIndexPrevious"), value);
        self
    }

    pub fn set_nf_token_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfNFTokenID"), value);
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

    pub fn build(self, index: Uint256) -> DirectoryNode {
        DirectoryNode::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for DirectoryNodeBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
