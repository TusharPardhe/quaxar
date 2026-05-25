//! Integration tests that pin `Transactor::minimumFee(...)` and its immediate
//! `checkFee(...)` bridge to the current C++ behavior.

use protocol::Ter;
use tx::{
    ApplyFlags, TransactorCheckFeeTx, run_transactor_check_fee_with_minimum_fee,
    run_transactor_minimum_fee,
};

#[test]
fn tx_transactor_minimum_fee_passes_unlimited_flag() {
    let observed = run_transactor_minimum_fee(
        "registry",
        10_u64,
        "fees",
        ApplyFlags::UNLIMITED,
        |base_fee, registry, fees, unlimited| {
            assert_eq!(base_fee, 10);
            assert_eq!(registry, "registry");
            assert_eq!(fees, "fees");
            assert!(unlimited);
            22_u64
        },
    );

    assert_eq!(observed, 22);
}

#[test]
fn tx_transactor_minimum_fee_clears_unlimited_flag() {
    let observed = run_transactor_minimum_fee(
        "registry",
        10_u64,
        "fees",
        ApplyFlags::NONE,
        |_, _, _, unlimited| {
            assert!(!unlimited);
            33_u64
        },
    );

    assert_eq!(observed, 33);
}

struct FeeTx {
    fee_is_native: bool,
    fee_paid: i64,
    fee_payer: &'static str,
}

impl TransactorCheckFeeTx for FeeTx {
    type AccountId = &'static str;
    type Amount = i64;

    fn fee_is_native(&self) -> bool {
        self.fee_is_native
    }

    fn fee_paid(&self) -> Self::Amount {
        self.fee_paid
    }

    fn fee_payer(&self) -> Self::AccountId {
        self.fee_payer
    }
}

#[test]
fn tx_transactor_check_fee_bridge_uses_minimum_fee_helper() {
    let result = run_transactor_check_fee_with_minimum_fee(
        ApplyFlags::UNLIMITED,
        true,
        &FeeTx {
            fee_is_native: true,
            fee_paid: 19,
            fee_payer: "alice",
        },
        10,
        0,
        &"registry",
        &"fees",
        |_| true,
        |base_fee, registry, fees, unlimited| {
            assert_eq!(base_fee, 10);
            assert_eq!(registry, &"registry");
            assert_eq!(fees, &"fees");
            assert!(unlimited);
            20
        },
        |_| Some(100_i64),
        |balance| *balance,
    );

    assert_eq!(result, Ter::TEL_INSUF_FEE_P);
}
