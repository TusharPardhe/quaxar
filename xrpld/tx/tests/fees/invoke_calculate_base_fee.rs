//! Integration tests that pin `applySteps.cpp::invoke_calculateBaseFee(...)`
//! to the current higher fee-dispatch behavior.

use protocol::{Rules, TxType};
use tx::{
    HasTxnType, run_invoke_calculate_base_fee_for_txn_source,
    run_invoke_calculate_base_fee_for_txn_type,
};

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

fn dispatch_for(txn_type: TxType) -> u64 {
    run_invoke_calculate_base_fee_for_txn_type(
        &Rules::new(std::iter::empty()),
        txn_type,
        || 0_u64,
        || 1_u64,
        || 2_u64,
        || 3_u64,
        || 4_u64,
        || 5_u64,
        || 6_u64,
        || 7_u64,
        || 8_u64,
        || 9_u64,
        || 10_u64,
    )
}

#[test]
fn tx_invoke_calculate_base_fee_routes_specialized_change_family() {
    assert_eq!(dispatch_for(TxType::AMENDMENT), 0);
    assert_eq!(dispatch_for(TxType::FEE), 0);
    assert_eq!(dispatch_for(TxType::UNL_MODIFY), 0);
}

#[test]
fn tx_invoke_calculate_base_fee_routes_default_for_unspecialized_types() {
    assert_eq!(dispatch_for(TxType::PAYMENT), 9);
}

#[test]
fn tx_invoke_calculate_base_fee_maps_unknown_txn_type_to_zero() {
    assert_eq!(dispatch_for(TxType::HOOK_SET), 10);
}

#[test]
fn tx_invoke_calculate_base_fee_source_wrapper_uses_txn_type() {
    let tx = StubTx {
        txn_type: TxType::ESCROW_FINISH,
    };

    let fee = run_invoke_calculate_base_fee_for_txn_source(
        &Rules::new(std::iter::empty()),
        &tx,
        || 0_u64,
        || 1_u64,
        || 2_u64,
        || 3_u64,
        || 4_u64,
        || 5_u64,
        || 6_u64,
        || 7_u64,
        || 8_u64,
        || 9_u64,
        || 10_u64,
    );

    assert_eq!(fee, 4);
}
