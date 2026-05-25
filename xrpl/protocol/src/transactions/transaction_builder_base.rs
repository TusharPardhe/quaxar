//! Shared transaction builder base mirroring `protocol_autogen/TransactionBuilderBase.h`.

use std::sync::Arc;

use crate::{
    AccountID, PublicKey, SField, STAmount, STArray, STObject, STTx, SecretKey, TxType,
    get_field_by_symbol, sign_st_object,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionBuilderBase {
    object: STObject,
}

impl TransactionBuilderBase {
    pub fn new(
        transaction_type: TxType,
        account: AccountID,
        sequence: Option<u32>,
        fee: Option<STAmount>,
    ) -> Self {
        let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
        object.set_field_u16(
            get_field_by_symbol("sfTransactionType"),
            transaction_type.to_u16(),
        );
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        if let Some(sequence) = sequence {
            object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        }
        if let Some(fee) = fee {
            object.set_field_amount(get_field_by_symbol("sfFee"), fee);
        }
        Self { object }
    }

    pub fn from_tx(tx: Arc<STTx>) -> Self {
        Self {
            object: tx.as_ref().clone_as_object(),
        }
    }

    pub fn object(&self) -> &STObject {
        &self.object
    }

    pub fn object_mut(&mut self) -> &mut STObject {
        &mut self.object
    }

    pub fn into_object(self) -> STObject {
        self.object
    }

    pub fn set_account(&mut self, value: AccountID) {
        self.object
            .set_account_id(get_field_by_symbol("sfAccount"), value);
    }

    pub fn set_fee(&mut self, value: STAmount) {
        self.object
            .set_field_amount(get_field_by_symbol("sfFee"), value);
    }

    pub fn set_sequence(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfSequence"), value);
    }

    pub fn set_ticket_sequence(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfSequence"), 0);
        self.object
            .set_field_u32(get_field_by_symbol("sfTicketSequence"), value);
    }

    pub fn set_flags(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfFlags"), value);
    }

    pub fn set_source_tag(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfSourceTag"), value);
    }

    pub fn set_last_ledger_sequence(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), value);
    }

    pub fn set_account_txn_id(&mut self, value: basics::base_uint::Uint256) {
        self.object
            .set_field_h256(get_field_by_symbol("sfAccountTxnID"), value);
    }

    pub fn set_previous_txn_id(&mut self, value: basics::base_uint::Uint256) {
        self.object
            .set_field_h256(get_field_by_symbol("sfPreviousTxnID"), value);
    }

    pub fn set_operation_limit(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfOperationLimit"), value);
    }

    pub fn set_memos(&mut self, value: STArray) {
        self.object
            .set_field_array(get_field_by_symbol("sfMemos"), value);
    }

    pub fn set_signers(&mut self, value: STArray) {
        self.object
            .set_field_array(get_field_by_symbol("sfSigners"), value);
    }

    pub fn set_network_id(&mut self, value: u32) {
        self.object
            .set_field_u32(get_field_by_symbol("sfNetworkID"), value);
    }

    pub fn set_delegate(&mut self, value: AccountID) {
        self.object
            .set_account_id(get_field_by_symbol("sfDelegate"), value);
    }

    pub fn set_blob_field(&mut self, field: &'static SField, value: &[u8]) {
        self.object.set_field_vl(field, value);
    }

    pub fn sign(
        &mut self,
        public_key: &PublicKey,
        secret_key: &SecretKey,
    ) -> Result<(), crate::SignError> {
        sign_st_object(
            &mut self.object,
            crate::HashPrefix::TxSign,
            public_key,
            secret_key,
            get_field_by_symbol("sfTxnSignature"),
        )
    }
}
