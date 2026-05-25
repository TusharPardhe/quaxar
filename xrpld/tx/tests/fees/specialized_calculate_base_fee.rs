//! Integration tests that pin the narrowed specialized
//! `calculateBaseFee(...)` selector to the current C++ custom-fee routing.

use protocol::TxType;
use tx::{
    HasTxnType, run_specialized_calculate_base_fee_for_txn_source,
    run_specialized_calculate_base_fee_for_txn_type,
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
    run_specialized_calculate_base_fee_for_txn_type(
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
    )
}

#[test]
fn tx_specialized_calculate_base_fee_routes_change_family() {
    assert_eq!(dispatch_for(TxType::AMENDMENT), 0);
    assert_eq!(dispatch_for(TxType::FEE), 0);
    assert_eq!(dispatch_for(TxType::UNL_MODIFY), 0);
}

#[test]
fn tx_specialized_calculate_base_fee_routes_landed_custom_fee_owners() {
    assert_eq!(dispatch_for(TxType::REGULAR_KEY_SET), 1);
    assert_eq!(dispatch_for(TxType::ACCOUNT_DELETE), 2);
    assert_eq!(dispatch_for(TxType::AMM_CREATE), 3);
    assert_eq!(dispatch_for(TxType::ESCROW_FINISH), 4);
    assert_eq!(dispatch_for(TxType::LOAN_SET), 5);
    assert_eq!(dispatch_for(TxType::LOAN_PAY), 6);
    assert_eq!(dispatch_for(TxType::BATCH), 7);
    assert_eq!(dispatch_for(TxType::LEDGER_STATE_FIX), 8);
}

#[test]
fn tx_specialized_calculate_base_fee_falls_back_to_default_for_unspecialized_types() {
    assert_eq!(dispatch_for(TxType::PAYMENT), 9);
}

#[test]
fn tx_specialized_calculate_base_fee_source_wrapper_uses_txn_type() {
    let tx = StubTx {
        txn_type: TxType::ESCROW_FINISH,
    };

    let fee = run_specialized_calculate_base_fee_for_txn_source(
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
    );

    assert_eq!(fee, 4);
}
