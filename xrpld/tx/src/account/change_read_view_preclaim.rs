//! Immutable typed preclaim for the `Change` pseudo-transaction family.
//!
//! This module owns only `ttAMENDMENT`, `ttFEE`, and `ttUNL_MODIFY`. It
//! mirrors `Change::preclaim` using the read-only `ReadView` and transaction
//! field-presence facts; it never invokes apply code or opens a sandbox.

use ledger::ReadView;
use protocol::{STTx, Ter, TxType, feature_xrp_fees, get_field_by_symbol};

use crate::{ChangePreclaimFacts, run_change_preclaim};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

/// Runs the complete immutable `Change::preclaim` tail for the three owned
/// pseudo-transaction types. `None` means `txn_type` is not Change-owned.
pub fn run_change_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    if view.open()
        && matches!(
            txn_type,
            TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY
        )
    {
        // `Change::preclaim` rejects the open ledger before it switches on the
        // transaction type or inspects any Fee fields.
        return Some(Ter::TEM_INVALID);
    }

    match txn_type {
        TxType::AMENDMENT | TxType::UNL_MODIFY => Some(Ter::TES_SUCCESS),
        TxType::FEE => Some(run_change_preclaim(
            TxType::FEE,
            ChangePreclaimFacts {
                ledger_is_open: false,
                xrp_fees_enabled: view.rules().enabled(&feature_xrp_fees()),
                base_fee_present: tx.is_field_present(sf("sfBaseFee")),
                reference_fee_units_present: tx.is_field_present(sf("sfReferenceFeeUnits")),
                reserve_base_present: tx.is_field_present(sf("sfReserveBase")),
                reserve_increment_present: tx.is_field_present(sf("sfReserveIncrement")),
                base_fee_drops_present: tx.is_field_present(sf("sfBaseFeeDrops")),
                reserve_base_drops_present: tx.is_field_present(sf("sfReserveBaseDrops")),
                reserve_increment_drops_present: tx.is_field_present(sf("sfReserveIncrementDrops")),
            },
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        Keylet, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount, feature_xrp_fees,
    };

    use super::{run_change_read_view_preclaim, sf};

    #[derive(Debug, Default)]
    struct View {
        open: bool,
        rules: Rules,
    }

    impl ReadView for View {
        fn open(&self) -> bool {
            self.open
        }

        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }

        fn fees(&self) -> Fees {
            Fees::default()
        }

        fn rules(&self) -> Rules {
            self.rules.clone()
        }

        fn exists(&self, _: Keylet) -> Result<bool, ViewError> {
            Ok(false)
        }

        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }

        fn read(&self, _: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(None)
        }

        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(Vec::new())
        }

        fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }

        fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }

        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn xrp_fee_tx() -> STTx {
        STTx::new(TxType::FEE, |tx| {
            for field in [
                "sfBaseFeeDrops",
                "sfReserveBaseDrops",
                "sfReserveIncrementDrops",
            ] {
                tx.set_field_amount(
                    sf(field),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                );
            }
        })
    }

    fn legacy_fee_tx() -> STTx {
        STTx::new(TxType::FEE, |tx| {
            tx.set_field_u64(sf("sfBaseFee"), 1);
            tx.set_field_u32(sf("sfReferenceFeeUnits"), 1);
            tx.set_field_u32(sf("sfReserveBase"), 1);
            tx.set_field_u32(sf("sfReserveIncrement"), 1);
        })
    }

    #[test]
    fn helper_returns_none_for_unowned_transaction_types() {
        let view = View::default();
        let tx = STTx::new(TxType::PAYMENT, |_| {});

        assert_eq!(
            run_change_read_view_preclaim(&view, &tx, TxType::PAYMENT),
            None
        );
    }

    #[test]
    fn open_ledger_rejection_precedes_fee_shape_validation() {
        let view = View {
            open: true,
            rules: Rules::new([feature_xrp_fees()]),
        };
        let malformed_fee = STTx::new(TxType::FEE, |_| {});

        assert_eq!(
            run_change_read_view_preclaim(&view, &malformed_fee, TxType::FEE),
            Some(Ter::TEM_INVALID)
        );
    }

    #[test]
    fn fee_field_shapes_preserve_rippled_required_before_forbidden_ordering() {
        let legacy_view = View::default();
        let missing_legacy_with_xrp_fields = xrp_fee_tx();
        let mut legacy_with_xrp_field = legacy_fee_tx();
        legacy_with_xrp_field.set_field_amount(
            sf("sfBaseFeeDrops"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );
        let xrp_view = View {
            open: false,
            rules: Rules::new([feature_xrp_fees()]),
        };
        let mut xrp_with_legacy_field = xrp_fee_tx();
        xrp_with_legacy_field.set_field_u64(sf("sfBaseFee"), 1);

        assert_eq!(
            run_change_read_view_preclaim(
                &legacy_view,
                &missing_legacy_with_xrp_fields,
                TxType::FEE,
            ),
            Some(Ter::TEM_MALFORMED),
            "legacy required fields fail before forbidden XRP fields"
        );
        assert_eq!(
            run_change_read_view_preclaim(&legacy_view, &legacy_with_xrp_field, TxType::FEE),
            Some(Ter::TEM_DISABLED)
        );
        assert_eq!(
            run_change_read_view_preclaim(&xrp_view, &xrp_with_legacy_field, TxType::FEE),
            Some(Ter::TEM_MALFORMED)
        );
    }

    #[test]
    fn amendment_and_unl_modify_are_explicit_success_tails() {
        let view = View::default();
        let amendment = STTx::new(TxType::AMENDMENT, |_| {});
        let unl_modify = STTx::new(TxType::UNL_MODIFY, |_| {});

        assert_eq!(
            run_change_read_view_preclaim(&view, &amendment, TxType::AMENDMENT),
            Some(Ter::TES_SUCCESS)
        );
        assert_eq!(
            run_change_read_view_preclaim(&view, &unl_modify, TxType::UNL_MODIFY),
            Some(Ter::TES_SUCCESS)
        );
    }
}
