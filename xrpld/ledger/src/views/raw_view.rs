//! Rust mutation-facing view seams mirroring `RawView.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{STLedgerEntry, Serializer, XRPAmount};

use crate::Ledger;
use crate::read_view::{TypedLedgerEntryRef, ViewError};

pub trait RawView {
    fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError>;
    fn raw_destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError>;
    /// Apply a batch of state map operations using a single MutableTree session.
    /// Default implementation falls back to individual operations.
    fn raw_apply_batch(
        &mut self,
        ops: &[(crate::StateBatchOp, Uint256, Vec<u8>)],
    ) -> Result<(), ViewError> {
        for (op, key, payload) in ops {
            match op {
                crate::StateBatchOp::Insert => {
                    let sle = Arc::new(protocol::STLedgerEntry::from_serial_iter(
                        &mut protocol::SerialIter::new(payload),
                        *key,
                    ));
                    self.raw_insert(sle)?;
                }
                crate::StateBatchOp::Update => {
                    let sle = Arc::new(protocol::STLedgerEntry::from_serial_iter(
                        &mut protocol::SerialIter::new(payload),
                        *key,
                    ));
                    self.raw_replace(sle)?;
                }
                crate::StateBatchOp::Delete => {
                    let sle = Arc::new(protocol::STLedgerEntry::from_type_and_key(
                        protocol::LedgerEntryType::Any,
                        *key,
                    ));
                    self.raw_erase(sle)?;
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
