//! Shared transaction wrapper base mirroring `protocol_autogen/TransactionBase.h`.

use std::sync::Arc;

use crate::{
    AccountID, STAmount, STTx, TxFormats, TxType, get_field_by_symbol, is_pseudo_tx,
    passes_local_checks, validate_st_object,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionBase {
    tx: Arc<STTx>,
}

impl TransactionBase {
    pub fn new(tx: Arc<STTx>) -> Self {
        Self { tx }
    }

    pub fn validate(&self, reason: &mut String) -> bool {
        let Some(format) = TxFormats::get_instance().find_by_type(self.tx.get_txn_type()) else {
            *reason = "Transaction failed schema validation".to_owned();
            return false;
        };

        if !validate_st_object(self.tx.as_ref(), format.so_template()) {
            *reason = "Transaction failed schema validation".to_owned();
            return false;
        }

        if is_pseudo_tx(self.tx.as_ref()) {
            return true;
        }

        match passes_local_checks(self.tx.as_ref()) {
            Ok(()) => true,
            Err(local_reason) => {
                *reason = local_reason;
                false
            }
        }
    }

    pub fn tx(&self) -> &Arc<STTx> {
        &self.tx
    }

    pub fn as_sttx(&self) -> &STTx {
        self.tx.as_ref()
    }

    pub fn get_transaction_type(&self) -> TxType {
        self.tx.get_txn_type()
    }

    pub fn get_account(&self) -> AccountID {
        self.tx.get_account_id(get_field_by_symbol("sfAccount"))
    }

    pub fn get_sequence(&self) -> u32 {
        self.tx.get_field_u32(get_field_by_symbol("sfSequence"))
    }

    pub fn get_fee(&self) -> STAmount {
        self.tx.get_field_amount(get_field_by_symbol("sfFee"))
    }

    pub fn get_signing_pub_key(&self) -> Vec<u8> {
        self.tx.get_field_vl(get_field_by_symbol("sfSigningPubKey"))
    }

    pub fn get_flags(&self) -> Option<u32> {
        self.tx
            .is_field_present(get_field_by_symbol("sfFlags"))
            .then(|| self.tx.get_field_u32(get_field_by_symbol("sfFlags")))
    }

    pub fn has_flags(&self) -> bool {
        self.tx.is_field_present(get_field_by_symbol("sfFlags"))
    }

    pub fn get_source_tag(&self) -> Option<u32> {
        self.tx
            .is_field_present(get_field_by_symbol("sfSourceTag"))
            .then(|| self.tx.get_field_u32(get_field_by_symbol("sfSourceTag")))
    }

    pub fn has_source_tag(&self) -> bool {
        self.tx.is_field_present(get_field_by_symbol("sfSourceTag"))
    }

    pub fn get_previous_txn_id(&self) -> Option<basics::base_uint::Uint256> {
        self.tx
            .is_field_present(get_field_by_symbol("sfPreviousTxnID"))
            .then(|| {
                self.tx
                    .get_field_h256(get_field_by_symbol("sfPreviousTxnID"))
            })
    }

    pub fn has_previous_txn_id(&self) -> bool {
        self.tx
            .is_field_present(get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_last_ledger_sequence(&self) -> Option<u32> {
        self.tx
            .is_field_present(get_field_by_symbol("sfLastLedgerSequence"))
            .then(|| {
                self.tx
                    .get_field_u32(get_field_by_symbol("sfLastLedgerSequence"))
            })
    }

    pub fn has_last_ledger_sequence(&self) -> bool {
        self.tx
            .is_field_present(get_field_by_symbol("sfLastLedgerSequence"))
    }

    pub fn get_account_txn_id(&self) -> Option<basics::base_uint::Uint256> {
        self.tx
            .is_field_present(get_field_by_symbol("sfAccountTxnID"))
            .then(|| {
                self.tx
                    .get_field_h256(get_field_by_symbol("sfAccountTxnID"))
            })
    }

    pub fn has_account_txn_id(&self) -> bool {
        self.tx
            .is_field_present(get_field_by_symbol("sfAccountTxnID"))
    }

    pub fn get_operation_limit(&self) -> Option<u32> {
        self.tx
            .is_field_present(get_field_by_symbol("sfOperationLimit"))
            .then(|| {
                self.tx
                    .get_field_u32(get_field_by_symbol("sfOperationLimit"))
            })
    }

    pub fn has_operation_limit(&self) -> bool {
        self.tx
            .is_field_present(get_field_by_symbol("sfOperationLimit"))
    }
}
