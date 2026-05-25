//! Narrow the reference implementation parity helpers.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use ledger::{Ledger, LedgerPersistenceRuntime, pend_save_validated};
use protocol::{STObject, STTx, SerialIter, TxMeta, get_field_by_symbol};

use crate::Transaction;

#[derive(Debug, Clone)]
pub struct AccountTxPageEntry {
    pub transaction: Arc<Transaction>,
    pub meta: TxMeta,
}

pub trait AccountTxPagingLedgerSource {
    fn get_ledger_by_seq(&self, seq: u32) -> Option<Arc<Ledger>>;
}

pub fn convert_blobs_to_tx_result(
    to: &mut Vec<AccountTxPageEntry>,
    ledger_index: u32,
    status: &str,
    raw_txn: &[u8],
    raw_meta: &[u8],
    network_id: u32,
) -> Result<(), String> {
    let txn = parse_transaction(raw_txn)?;
    let mut transaction = Transaction::new(Arc::new(txn));
    let (meta, has_transaction_index) = parse_meta(transaction.get_id(), ledger_index, raw_meta)?;
    let meta_object = meta.get_as_object();
    let txn_index_field = get_field_by_symbol("sfTransactionIndex");
    let txn_status = Transaction::sql_transaction_status(Some(status));

    if has_transaction_index {
        transaction.set_status_with_ledger(
            txn_status,
            ledger_index,
            Some(meta_object.get_field_u32(txn_index_field)),
            Some(network_id),
        );
    } else {
        transaction.set_status_with_ledger(txn_status, ledger_index, None, None);
    }

    to.push(AccountTxPageEntry {
        transaction: Arc::new(transaction),
        meta,
    });
    Ok(())
}

pub fn save_ledger_async<S>(
    ledger_source: &S,
    persistence: Arc<dyn LedgerPersistenceRuntime>,
    seq: u32,
) -> bool
where
    S: AccountTxPagingLedgerSource,
{
    ledger_source
        .get_ledger_by_seq(seq)
        .is_some_and(|ledger| pend_save_validated(persistence, ledger, false, false))
}

fn parse_transaction(raw_txn: &[u8]) -> Result<STTx, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(raw_txn);
        STTx::from_serial_iter(&mut serial)
    }))
    .map_err(unwind_message)
}

fn parse_meta(
    transaction_id: basics::base_uint::Uint256,
    ledger_index: u32,
    raw_meta: &[u8],
) -> Result<(TxMeta, bool), String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut sit = SerialIter::new(raw_meta);
        let mut object = STObject::from_serial_iter(&mut sit, get_field_by_symbol("sfMetadata"), 0);
        let txn_index_field = get_field_by_symbol("sfTransactionIndex");
        let has_transaction_index = object.is_field_present(txn_index_field);
        if !has_transaction_index {
            object.set_field_u32(txn_index_field, 0);
        }
        (
            TxMeta::from_stobject(transaction_id, ledger_index, object),
            has_transaction_index,
        )
    }))
    .map_err(unwind_message)
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    "failed to decode account_tx paging blobs".to_owned()
}

#[cfg(test)]
mod tests {
    use super::{AccountTxPagingLedgerSource, convert_blobs_to_tx_result, save_ledger_async};
    use std::sync::{Arc, Mutex};

    use ledger::{
        Ledger, LedgerPersistenceJob, LedgerPersistenceJobType, LedgerPersistenceRuntime,
    };
    use protocol::{
        JsonOptions, JsonValue, STAmount, STArray, STObject, STTx, TxMeta, TxType,
        get_field_by_symbol,
    };

    fn sample_tx() -> STTx {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 7);
        })
    }

    fn meta_for(tx_id: basics::base_uint::Uint256, ledger_seq: u32, include_index: bool) -> TxMeta {
        let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
        object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
        if include_index {
            object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
        }
        object.set_field_array(
            get_field_by_symbol("sfAffectedNodes"),
            STArray::new(get_field_by_symbol("sfAffectedNodes")),
        );
        TxMeta::from_stobject(tx_id, ledger_seq, object)
    }

    fn raw_meta_without_index() -> Vec<u8> {
        let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
        object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
        object.set_field_array(
            get_field_by_symbol("sfAffectedNodes"),
            STArray::new(get_field_by_symbol("sfAffectedNodes")),
        );
        object.get_serializer().data().to_vec()
    }

    #[derive(Default)]
    struct FakeLedgerSource {
        ledger: Option<Arc<Ledger>>,
    }

    impl AccountTxPagingLedgerSource for FakeLedgerSource {
        fn get_ledger_by_seq(&self, seq: u32) -> Option<Arc<Ledger>> {
            self.ledger
                .as_ref()
                .filter(|ledger| ledger.header().seq == seq)
                .cloned()
        }
    }

    #[derive(Default)]
    struct RecordingPersistence {
        events: Mutex<Vec<String>>,
    }

    impl LedgerPersistenceRuntime for RecordingPersistence {
        fn mark_saved(&self, hash: basics::sha_map_hash::SHAMapHash) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("mark:{hash}"));
            true
        }

        fn start_work(&self, seq: u32) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("start:{seq}"));
            true
        }

        fn finish_work(&self, seq: u32) {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("finish:{seq}"));
        }

        fn should_work(&self, seq: u32, is_synchronous: bool) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("should:{seq}:{is_synchronous}"));
            true
        }

        fn pending(&self, seq: u32) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("pending:{seq}"));
            false
        }

        fn save_validated_ledger(&self, ledger: Arc<Ledger>, is_current: bool) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("save:{}:{is_current}", ledger.header().seq));
            true
        }

        fn enqueue_job(
            &self,
            job_type: LedgerPersistenceJobType,
            job_name: String,
            job: LedgerPersistenceJob,
        ) -> bool {
            self.events
                .lock()
                .expect("events mutex")
                .push(format!("enqueue:{job_type:?}:{job_name}"));
            job();
            true
        }
    }

    #[test]
    fn convert_blobs_to_tx_result_sets_ctid_when_meta_has_transaction_index() {
        let tx = sample_tx();
        let meta = meta_for(tx.get_transaction_id(), 44, true);
        let mut rows = Vec::new();

        convert_blobs_to_tx_result(
            &mut rows,
            44,
            "V",
            tx.get_serializer().data(),
            meta.get_as_object().get_serializer().data(),
            2,
        )
        .expect("blob conversion should succeed");

        assert_eq!(rows.len(), 1);
        let json = rows[0]
            .transaction
            .get_json(JsonOptions::DISABLE_API_PRIOR_V2, false);
        let JsonValue::Object(object) = json else {
            panic!("transaction json must be an object");
        };
        assert_eq!(rows[0].transaction.get_ledger(), 44);
        assert_eq!(
            rows[0].transaction.get_status(),
            crate::TransStatus::COMMITTED
        );
        assert_eq!(rows[0].meta.get_index(), 3);
        assert!(object.contains_key("ctid"));
    }

    #[test]
    fn convert_blobs_to_tx_result_skips_ctid_when_meta_lacks_transaction_index() {
        let tx = sample_tx();
        let mut rows = Vec::new();

        convert_blobs_to_tx_result(
            &mut rows,
            44,
            "V",
            tx.get_serializer().data(),
            &raw_meta_without_index(),
            2,
        )
        .expect("blob conversion should succeed");

        let json = rows[0]
            .transaction
            .get_json(JsonOptions::DISABLE_API_PRIOR_V2, false);
        let JsonValue::Object(object) = json else {
            panic!("transaction json must be an object");
        };
        assert!(!object.contains_key("ctid"));
    }

    #[test]
    fn save_ledger_async_only_persists_existing_immutable_ledgers() {
        let mut ledger = Ledger::from_ledger_seq_and_close_time(77, 123, true);
        ledger.set_immutable(true);
        let source = FakeLedgerSource {
            ledger: Some(Arc::new(ledger)),
        };
        let persistence = Arc::new(RecordingPersistence::default());

        assert!(save_ledger_async(&source, persistence.clone(), 77));
        assert!(!save_ledger_async(&source, persistence.clone(), 66));

        let events = persistence.events.lock().expect("events mutex");
        assert!(
            events
                .iter()
                .any(|event| event == "enqueue:PubOldLedger:Pub77")
        );
        assert!(events.iter().any(|event| event == "save:77:false"));
    }
}
