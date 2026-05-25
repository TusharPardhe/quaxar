use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRoot {
    base: crate::LedgerEntryBase,
}

impl AccountRoot {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::AccountRoot;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for AccountRoot".to_owned());
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

    pub fn get_sequence(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSequence"))
    }

    pub fn get_balance(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfBalance"))
    }

    pub fn get_owner_count(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfOwnerCount"))
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

    pub fn get_account_txn_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_account_txn_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfAccountTxnID"))
        })
    }

    pub fn has_account_txn_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAccountTxnID"))
    }

    pub fn get_regular_key(&self) -> Option<crate::AccountID> {
        self.has_regular_key().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_account_id(crate::get_field_by_symbol("sfRegularKey"))
        })
    }

    pub fn has_regular_key(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfRegularKey"))
    }

    pub fn get_email_hash(&self) -> Option<basics::base_uint::Uint128> {
        self.has_email_hash().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h128(crate::get_field_by_symbol("sfEmailHash"))
        })
    }

    pub fn has_email_hash(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfEmailHash"))
    }

    pub fn get_wallet_locator(&self) -> Option<basics::base_uint::Uint256> {
        self.has_wallet_locator().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfWalletLocator"))
        })
    }

    pub fn has_wallet_locator(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfWalletLocator"))
    }

    pub fn get_wallet_size(&self) -> Option<u32> {
        self.has_wallet_size().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfWalletSize"))
        })
    }

    pub fn has_wallet_size(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfWalletSize"))
    }

    pub fn get_message_key(&self) -> Option<Vec<u8>> {
        self.has_message_key().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfMessageKey"))
        })
    }

    pub fn has_message_key(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMessageKey"))
    }

    pub fn get_transfer_rate(&self) -> Option<u32> {
        self.has_transfer_rate().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfTransferRate"))
        })
    }

    pub fn has_transfer_rate(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTransferRate"))
    }

    pub fn get_domain(&self) -> Option<Vec<u8>> {
        self.has_domain().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfDomain"))
        })
    }

    pub fn has_domain(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDomain"))
    }

    pub fn get_tick_size(&self) -> Option<u8> {
        self.has_tick_size().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u8(crate::get_field_by_symbol("sfTickSize"))
        })
    }

    pub fn has_tick_size(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTickSize"))
    }

    pub fn get_ticket_count(&self) -> Option<u32> {
        self.has_ticket_count().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfTicketCount"))
        })
    }

    pub fn has_ticket_count(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfTicketCount"))
    }

    pub fn get_nf_token_minter(&self) -> Option<crate::AccountID> {
        self.has_nf_token_minter().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_account_id(crate::get_field_by_symbol("sfNFTokenMinter"))
        })
    }

    pub fn has_nf_token_minter(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfNFTokenMinter"))
    }

    pub fn get_minted_nf_tokens(&self) -> Option<u32> {
        self.has_minted_nf_tokens().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfMintedNFTokens"))
        })
    }

    pub fn has_minted_nf_tokens(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfMintedNFTokens"))
    }

    pub fn get_burned_nf_tokens(&self) -> Option<u32> {
        self.has_burned_nf_tokens().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfBurnedNFTokens"))
        })
    }

    pub fn has_burned_nf_tokens(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfBurnedNFTokens"))
    }

    pub fn get_first_nf_token_sequence(&self) -> Option<u32> {
        self.has_first_nf_token_sequence().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfFirstNFTokenSequence"))
        })
    }

    pub fn has_first_nf_token_sequence(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfFirstNFTokenSequence"))
    }

    pub fn get_ammid(&self) -> Option<basics::base_uint::Uint256> {
        self.has_ammid().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfAMMID"))
        })
    }

    pub fn has_ammid(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfAMMID"))
    }

    pub fn get_vault_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_vault_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfVaultID"))
        })
    }

    pub fn has_vault_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfVaultID"))
    }

    pub fn get_loan_broker_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_loan_broker_id().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"))
        })
    }

    pub fn has_loan_broker_id(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfLoanBrokerID"))
    }
}

impl Deref for AccountRoot {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRootBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl AccountRootBuilder {
    pub fn new(
        account: crate::AccountID,
        sequence: u32,
        balance: crate::STAmount,
        owner_count: u32,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(AccountRoot::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_sequence(sequence);
        builder = builder.set_balance(balance);
        builder = builder.set_owner_count(owner_count);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != AccountRoot::ENTRY_TYPE {
            return Err("Invalid ledger entry type for AccountRoot".to_owned());
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

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSequence"), value);
        self
    }

    pub fn set_balance(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfBalance"), value);
        self
    }

    pub fn set_owner_count(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfOwnerCount"), value);
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

    pub fn set_account_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfAccountTxnID"), value);
        self
    }

    pub fn set_regular_key(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfRegularKey"), value);
        self
    }

    pub fn set_email_hash(mut self, value: basics::base_uint::Uint128) -> Self {
        self.base
            .object_mut()
            .set_field_h128(crate::get_field_by_symbol("sfEmailHash"), value);
        self
    }

    pub fn set_wallet_locator(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfWalletLocator"), value);
        self
    }

    pub fn set_wallet_size(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfWalletSize"), value);
        self
    }

    pub fn set_message_key(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfMessageKey"), value.as_ref());
        self
    }

    pub fn set_transfer_rate(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfTransferRate"), value);
        self
    }

    pub fn set_domain(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfDomain"), value.as_ref());
        self
    }

    pub fn set_tick_size(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfTickSize"), value);
        self
    }

    pub fn set_ticket_count(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfTicketCount"), value);
        self
    }

    pub fn set_nf_token_minter(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfNFTokenMinter"), value);
        self
    }

    pub fn set_minted_nf_tokens(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfMintedNFTokens"), value);
        self
    }

    pub fn set_burned_nf_tokens(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfBurnedNFTokens"), value);
        self
    }

    pub fn set_first_nf_token_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfFirstNFTokenSequence"), value);
        self
    }

    pub fn set_ammid(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfAMMID"), value);
        self
    }

    pub fn set_vault_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfVaultID"), value);
        self
    }

    pub fn set_loan_broker_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfLoanBrokerID"), value);
        self
    }

    pub fn build(self, index: Uint256) -> AccountRoot {
        AccountRoot::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for AccountRootBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
