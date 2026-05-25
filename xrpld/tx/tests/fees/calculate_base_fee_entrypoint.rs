//! Integration tests that pin the public `applySteps.cpp` fee-entry wrappers
//! to the current C++ delegation shape.

use tx::{run_calculate_base_fee_entrypoint, run_calculate_default_base_fee_entrypoint};

#[test]
fn tx_calculate_base_fee_entrypoint_delegates_to_invoke_shell() {
    let fee = run_calculate_base_fee_entrypoint("view", "tx", |view, tx| {
        assert_eq!(view, "view");
        assert_eq!(tx, "tx");
        12_u64
    });

    assert_eq!(fee, 12_u64);
}

#[test]
fn tx_calculate_default_base_fee_entrypoint_delegates_to_default_shell() {
    let fee = run_calculate_default_base_fee_entrypoint("view", "tx", |view, tx| {
        assert_eq!(view, "view");
        assert_eq!(tx, "tx");
        9_u64
    });

    assert_eq!(fee, 9_u64);
}
