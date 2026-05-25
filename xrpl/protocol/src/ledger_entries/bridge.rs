use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bridge {
    base: crate::LedgerEntryBase,
}

impl Bridge {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Bridge;

    #[allow(clippy::too_many_arguments)]
    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Bridge".to_owned());
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

    pub fn get_signature_reward(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfSignatureReward"))
    }

    pub fn get_min_account_create_amount(&self) -> Option<crate::STAmount> {
        self.has_min_account_create_amount().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_amount(crate::get_field_by_symbol("sfMinAccountCreateAmount"))
        })
    }

    pub fn has_min_account_create_amount(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMinAccountCreateAmount"))
    }

    pub fn get_x_chain_bridge(&self) -> crate::STXChainBridge {
        self.base
            .as_st_ledger_entry()
            .get_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"))
    }

    pub fn get_x_chain_claim_id(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfXChainClaimID"))
    }

    pub fn get_x_chain_account_create_count(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfXChainAccountCreateCount"))
    }

    pub fn get_x_chain_account_claim_count(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfXChainAccountClaimCount"))
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

impl Deref for Bridge {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl BridgeBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        signature_reward: crate::STAmount,
        x_chain_bridge: crate::STXChainBridge,
        x_chain_claim_id: u64,
        x_chain_account_create_count: u64,
        x_chain_account_claim_count: u64,
        owner_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Bridge::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_signature_reward(signature_reward);
        builder = builder.set_x_chain_bridge(x_chain_bridge);
        builder = builder.set_x_chain_claim_id(x_chain_claim_id);
        builder = builder.set_x_chain_account_create_count(x_chain_account_create_count);
        builder = builder.set_x_chain_account_claim_count(x_chain_account_claim_count);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Bridge::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Bridge".to_owned());
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

    pub fn set_signature_reward(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfSignatureReward"), value);
        self
    }

    pub fn set_min_account_create_amount(mut self, value: crate::STAmount) -> Self {
        self.base.object_mut().set_field_amount(
            crate::get_field_by_symbol("sfMinAccountCreateAmount"),
            value,
        );
        self
    }

    pub fn set_x_chain_bridge(mut self, value: crate::STXChainBridge) -> Self {
        self.base
            .object_mut()
            .set_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"), value);
        self
    }

    pub fn set_x_chain_claim_id(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfXChainClaimID"), value);
        self
    }

    pub fn set_x_chain_account_create_count(mut self, value: u64) -> Self {
        self.base.object_mut().set_field_u64(
            crate::get_field_by_symbol("sfXChainAccountCreateCount"),
            value,
        );
        self
    }

    pub fn set_x_chain_account_claim_count(mut self, value: u64) -> Self {
        self.base.object_mut().set_field_u64(
            crate::get_field_by_symbol("sfXChainAccountClaimCount"),
            value,
        );
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

    pub fn build(self, index: Uint256) -> Bridge {
        Bridge::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for BridgeBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
