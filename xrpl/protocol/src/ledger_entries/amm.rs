use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMM {
    base: crate::LedgerEntryBase,
}

impl AMM {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::AMM;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for AMM".to_owned());
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

    pub fn get_trading_fee(&self) -> Option<u16> {
        self.has_trading_fee().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u16(crate::get_field_by_symbol("sfTradingFee"))
        })
    }

    pub fn has_trading_fee(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTradingFee"))
    }

    pub fn get_vote_slots(&self) -> Option<&crate::STArray> {
        self.base
            .as_st_ledger_entry()
            .peek_at_pfield(crate::get_field_by_symbol("sfVoteSlots"))
            .and_then(|value| value.as_any().downcast_ref::<crate::STArray>())
    }

    pub fn has_vote_slots(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfVoteSlots"))
    }

    pub fn get_auction_slot(&self) -> Option<crate::STObject> {
        self.has_auction_slot().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_object(crate::get_field_by_symbol("sfAuctionSlot"))
        })
    }

    pub fn has_auction_slot(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAuctionSlot"))
    }

    pub fn get_lp_token_balance(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfLPTokenBalance"))
    }

    pub fn get_asset(&self) -> crate::STIssue {
        self.base
            .as_st_ledger_entry()
            .get_field_issue(crate::get_field_by_symbol("sfAsset"))
    }

    pub fn get_asset2(&self) -> crate::STIssue {
        self.base
            .as_st_ledger_entry()
            .get_field_issue(crate::get_field_by_symbol("sfAsset2"))
    }

    pub fn get_owner_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfOwnerNode"))
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
}

impl Deref for AMM {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl AMMBuilder {
    pub fn new(
        account: crate::AccountID,
        lp_token_balance: crate::STAmount,
        asset: crate::STIssue,
        asset2: crate::STIssue,
        owner_node: u64,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(AMM::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_lp_token_balance(lp_token_balance);
        builder = builder.set_asset(asset);
        builder = builder.set_asset2(asset2);
        builder = builder.set_owner_node(owner_node);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != AMM::ENTRY_TYPE {
            return Err("Invalid ledger entry type for AMM".to_owned());
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

    pub fn set_trading_fee(mut self, value: u16) -> Self {
        self.base
            .object_mut()
            .set_field_u16(crate::get_field_by_symbol("sfTradingFee"), value);
        self
    }

    pub fn set_vote_slots(mut self, value: crate::STArray) -> Self {
        self.base
            .object_mut()
            .set_field_array(crate::get_field_by_symbol("sfVoteSlots"), value);
        self
    }

    pub fn set_auction_slot(mut self, value: crate::STObject) -> Self {
        self.base
            .object_mut()
            .set_field_object(crate::get_field_by_symbol("sfAuctionSlot"), value);
        self
    }

    pub fn set_lp_token_balance(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfLPTokenBalance"), value);
        self
    }

    pub fn set_asset(mut self, value: crate::STIssue) -> Self {
        self.base
            .object_mut()
            .set_field_issue(crate::get_field_by_symbol("sfAsset"), value);
        self
    }

    pub fn set_asset2(mut self, value: crate::STIssue) -> Self {
        self.base
            .object_mut()
            .set_field_issue(crate::get_field_by_symbol("sfAsset2"), value);
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

    pub fn build(self, index: Uint256) -> AMM {
        AMM::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for AMMBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
