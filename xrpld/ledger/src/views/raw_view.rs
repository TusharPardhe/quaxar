//! Rust mutation-facing view seams mirroring `RawView.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{STLedgerEntry, Serializer, XRPAmount};

use crate::Ledger;
use crate::read_view::{ReadView, TypedLedgerEntryRef, ViewError};

fn decode_batch_sle(payload: &[u8], key: Uint256) -> Result<Arc<STLedgerEntry>, ViewError> {
    if payload.is_empty() {
        return Err(ViewError::Conversion(
            "state batch operation omitted serialized ledger entry".to_owned(),
        ));
    }
    let mut serial = protocol::SerialIter::new(payload);
    let sle = STLedgerEntry::try_from_serial_iter(&mut serial, key).map_err(|error| {
        ViewError::Conversion(format!(
            "state batch ledger entry could not be decoded: {error:?}"
        ))
    })?;
    if !serial.empty() || sle.get_serializer().data() != payload {
        return Err(ViewError::Conversion(
            "state batch ledger entry is not a canonical standalone encoding".to_owned(),
        ));
    }
    Ok(Arc::new(sle))
}

/// A mutable view that can also resolve its current ledger state.
///
/// Transaction threading uses this to add AccountRoot modifications for owners
/// of deleted ledger entries that were not otherwise changed by the transaction.
pub trait ReadRawView: ReadView + RawView {}

impl<T> ReadRawView for T where T: ReadView + RawView + ?Sized {}

pub trait RawView {
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError>;
    fn raw_apply_sle_batch(
        &mut self,
        ops: &[(crate::StateBatchOp, Arc<STLedgerEntry>)],
    ) -> Result<(), ViewError> {
        for (op, sle) in ops {
            match op {
                crate::StateBatchOp::Insert => self.raw_insert(Arc::clone(sle))?,
                crate::StateBatchOp::Update => self.raw_replace(Arc::clone(sle))?,
                crate::StateBatchOp::Delete => self.raw_erase(Arc::clone(sle))?,
            }
        }
        Ok(())
    }
    /// Apply a batch of state map operations using a single MutableTree session.
    /// Default implementation falls back to individual operations.
    fn raw_apply_batch(
        &mut self,
        ops: &[(crate::StateBatchOp, Uint256, Vec<u8>)],
    ) -> Result<(), ViewError> {
        for (op, key, payload) in ops {
            match op {
                crate::StateBatchOp::Insert => {
                    self.raw_insert(decode_batch_sle(payload, *key)?)?;
                }
                crate::StateBatchOp::Update => {
                    self.raw_replace(decode_batch_sle(payload, *key)?)?;
                }
                crate::StateBatchOp::Delete => {
                    self.raw_erase(decode_batch_sle(payload, *key)?)?;
                }
            }
        }
        Ok(())
    }
}

pub trait TxsRawView: RawView {
    fn raw_tx_insert(
        &mut self,
        key: Uint256,
        txn: Arc<Serializer>,
        metadata: Option<Arc<Serializer>>,
    ) -> Result<(), ViewError>;
}

pub trait TypedRawViewExt: RawView {
    fn raw_erase_typed<T>(&mut self, sle: &T) -> Result<(), ViewError>
    where
        T: TypedLedgerEntryRef,
    {
        self.raw_erase(sle.sle())
    }

    fn raw_insert_typed<T>(&mut self, sle: &T) -> Result<(), ViewError>
    where
        T: TypedLedgerEntryRef,
    {
        self.raw_insert(sle.sle())
    }

    fn raw_replace_typed<T>(&mut self, sle: &T) -> Result<(), ViewError>
    where
        T: TypedLedgerEntryRef,
    {
        self.raw_replace(sle.sle())
    }
}

impl<T> TypedRawViewExt for T where T: RawView + ?Sized {}

impl RawView for Ledger {
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.delete_state_map_item(*sle.key())?;
        Ok(())
    }

    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.insert_state_map_item(*sle.key(), sle.get_serializer().data().to_vec())?;
        Ok(())
    }

    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
        self.update_state_map_item(*sle.key(), sle.get_serializer().data().to_vec())?;
        Ok(())
    }

    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
        if fee.drops() < 0 {
            return Err(ViewError::InvalidFee(fee));
        }
        self.set_total_drops(
            self.header()
                .drops
                .checked_sub(fee.drops() as u64)
                .ok_or(ViewError::InvalidFee(fee))?,
        );
        Ok(())
    }

    fn raw_apply_sle_batch(
        &mut self,
        ops: &[(crate::StateBatchOp, Arc<STLedgerEntry>)],
    ) -> Result<(), ViewError> {
        let serialized = ops
            .iter()
            .map(|(op, sle)| (*op, *sle.key(), sle.get_serializer().data().to_vec()))
            .collect::<Vec<_>>();
        self.apply_state_batch(&serialized)
            .map_err(ViewError::Mutation)
    }

    fn raw_apply_batch(
        &mut self,
        ops: &[(crate::StateBatchOp, Uint256, Vec<u8>)],
    ) -> Result<(), ViewError> {
        self.apply_state_batch(ops).map_err(ViewError::Mutation)
    }
}

impl TxsRawView for Ledger {
    fn raw_tx_insert(
        &mut self,
        key: Uint256,
        txn: Arc<Serializer>,
        metadata: Option<Arc<Serializer>>,
    ) -> Result<(), ViewError> {
        let metadata = metadata.ok_or(ViewError::MissingMetadata(key))?;
        self.insert_tx_map_item(key, txn.data().to_vec(), metadata.data().to_vec())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use basics::base_uint::Uint256;
    use protocol::{LedgerEntryType, STLedgerEntry, XRPAmount};

    use super::{RawView, ViewError};
    use crate::StateBatchOp;

    #[derive(Default)]
    struct RecordingRawView {
        erased_type: Option<LedgerEntryType>,
    }

    impl RawView for RecordingRawView {
        fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.erased_type = Some(sle.get_type());
            Ok(())
        }

        fn raw_insert(&mut self, _sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            unreachable!("delete-only regression")
        }

        fn raw_replace(&mut self, _sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            unreachable!("delete-only regression")
        }

        fn raw_destroy_xrp(&mut self, _fee: XRPAmount) -> Result<(), ViewError> {
            Ok(())
        }
    }

    #[test]
    fn batch_delete_rejects_empty_or_malformed_payload_without_panicking() {
        let key = Uint256::from_array([0x5A; 32]);
        let mut erased = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, key);
        erased.set_field_h256(
            protocol::get_field_by_symbol("sfPreviousTxnID"),
            Uint256::zero(),
        );
        let mut valid_payload_with_trailing_object_end = erased.get_serializer().data().to_vec();
        // `0xE1` is an STObject end marker. The permissive object decoder consumes
        // it, so canonical round-trip validation must reject it explicitly.
        valid_payload_with_trailing_object_end.push(0xE1);
        let mut invalid_ledger_entry_type = erased.get_serializer().data().to_vec();
        assert_eq!(
            invalid_ledger_entry_type[0], 0x11,
            "serialized SLE must begin with sfLedgerEntryType"
        );
        invalid_ledger_entry_type[1..3].copy_from_slice(&0xFFFFu16.to_be_bytes());
        let mut view = RecordingRawView::default();

        for payload in [
            Vec::new(),
            vec![0xFF],
            invalid_ledger_entry_type,
            valid_payload_with_trailing_object_end,
        ] {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                view.raw_apply_batch(&[(StateBatchOp::Delete, key, payload)])
            }));
            assert!(result.is_ok(), "malformed delete must not unwind");
            assert!(result.expect("caught result").is_err());
        }
        assert_eq!(view.erased_type, None);
    }

    #[test]
    fn batch_delete_preserves_erased_sle_type_for_generic_raw_views() {
        let key = Uint256::from_array([0xA5; 32]);
        let erased = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, key);
        let payload = erased.get_serializer().data().to_vec();
        let mut view = RecordingRawView::default();

        view.raw_apply_batch(&[(StateBatchOp::Delete, key, payload)])
            .expect("typed delete payload should apply without an Any SLE");

        assert_eq!(view.erased_type, Some(LedgerEntryType::Offer));
    }
}
