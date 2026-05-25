use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayChannel {
    base: crate::LedgerEntryBase,
}

impl PayChannel {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::PayChannel;

    #[allow(clippy::too_many_arguments)]
    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for PayChannel".to_owned());
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

    pub fn get_destination(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfDestination"))
    }

    pub fn get_sequence(&self) -> Option<u32> {
        self.has_sequence().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfSequence"))
        })
    }

    pub fn has_sequence(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfSequence"))
    }

    pub fn get_amount(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_balance(&self) -> crate::STAmount {
        self.base
            .as_st_ledger_entry()
            .get_field_amount(crate::get_field_by_symbol("sfBalance"))
    }

    pub fn get_public_key(&self) -> Vec<u8> {
        self.base
            .as_st_ledger_entry()
            .get_field_vl(crate::get_field_by_symbol("sfPublicKey"))
    }

    pub fn get_settle_delay(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfSettleDelay"))
    }

    pub fn get_expiration(&self) -> Option<u32> {
        self.has_expiration().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfExpiration"))
        })
    }

    pub fn has_expiration(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfExpiration"))
    }

    pub fn get_cancel_after(&self) -> Option<u32> {
        self.has_cancel_after().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfCancelAfter"))
        })
    }

    pub fn has_cancel_after(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfCancelAfter"))
    }

    pub fn get_source_tag(&self) -> Option<u32> {
        self.has_source_tag().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfSourceTag"))
        })
    }

    pub fn has_source_tag(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfSourceTag"))
    }

    pub fn get_destination_tag(&self) -> Option<u32> {
        self.has_destination_tag().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfDestinationTag"))
        })
    }

    pub fn has_destination_tag(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDestinationTag"))
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

    pub fn get_destination_node(&self) -> Option<u64> {
        self.has_destination_node().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfDestinationNode"))
        })
    }

    pub fn has_destination_node(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfDestinationNode"))
    }
}

impl Deref for PayChannel {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayChannelBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl PayChannelBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        destination: crate::AccountID,
        amount: crate::STAmount,
        balance: crate::STAmount,
        public_key: impl AsRef<[u8]>,
        settle_delay: u32,
        owner_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(PayChannel::ENTRY_TYPE),
        };
        builder = builder.set_account(account);
        builder = builder.set_destination(destination);
        builder = builder.set_amount(amount);
        builder = builder.set_balance(balance);
        builder = builder.set_public_key(public_key);
        builder = builder.set_settle_delay(settle_delay);
        builder = builder.set_owner_node(owner_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != PayChannel::ENTRY_TYPE {
            return Err("Invalid ledger entry type for PayChannel".to_owned());
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

    pub fn set_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfDestination"), value);
        self
    }

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSequence"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_balance(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfBalance"), value);
        self
    }

    pub fn set_public_key(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfPublicKey"), value.as_ref());
        self
    }

    pub fn set_settle_delay(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSettleDelay"), value);
        self
    }

    pub fn set_expiration(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfExpiration"), value);
        self
    }

    pub fn set_cancel_after(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfCancelAfter"), value);
        self
    }

    pub fn set_source_tag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfSourceTag"), value);
        self
    }

    pub fn set_destination_tag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfDestinationTag"), value);
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

    pub fn set_destination_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfDestinationNode"), value);
        self
    }

    pub fn build(self, index: Uint256) -> PayChannel {
        PayChannel::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for PayChannelBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
