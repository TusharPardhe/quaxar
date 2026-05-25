//! Integration tests that pin the vault-family preflight dispatch shell to the
//! current C++ vault transaction behavior.

use std::cell::{Cell, RefCell};

use protocol::{INNER_BATCH_TRANSACTION_FLAG, Ter, TxType};
use tx::{
    HasTxnType, UnknownTransactionType, VAULT_BASE_FLAGS_MASK,
    VAULT_FULLY_CANONICAL_SIGNATURE_FLAG, get_vault_flags_mask_for_txn_source,
    get_vault_flags_mask_for_txn_type, run_vault_check_extra_features_for_txn_source,
    run_vault_check_extra_features_for_txn_type, run_vault_preflight_for_txn_source,
    run_vault_preflight_for_txn_type,
};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_preflight_dispatch_short_circuits_shared_feature_gate_macro() {
    let trace = RefCell::new(Vec::new());

    let result = run_vault_preflight_for_txn_type(
        false,
        TxType::VAULT_CREATE,
        || {
            trace.borrow_mut().push("create-extra");
            true
        },
        || {
            trace.borrow_mut().push("set-extra");
            true
        },
        || {
            trace.borrow_mut().push("create-preflight");
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ok(Ter::TEM_DISABLED));
    assert!(trace.borrow().is_empty());
}

#[test]
fn vault_preflight_dispatch_runs_selected_extra_features_then_selected_preflight() {
    let trace = RefCell::new(Vec::new());

    let result = run_vault_preflight_for_txn_type(
        true,
        TxType::VAULT_SET,
        || {
            trace.borrow_mut().push("create-extra");
            true
        },
        || {
            trace.borrow_mut().push("set-extra");
            true
        },
        || Ter::TES_SUCCESS,
        || {
            trace.borrow_mut().push("set-preflight");
            Ter::TEM_MALFORMED
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ok(Ter::TEM_MALFORMED));
    assert_eq!(trace.into_inner(), vec!["set-extra", "set-preflight"]);
}

#[test]
fn vault_preflight_dispatch_skips_extra_feature_wrappers_for_other_vault_types() {
    let create_called = Cell::new(false);
    let set_called = Cell::new(false);

    let extra = run_vault_check_extra_features_for_txn_type(
        TxType::VAULT_WITHDRAW,
        || {
            create_called.set(true);
            false
        },
        || {
            set_called.set(true);
            false
        },
    );

    assert_eq!(extra, Ok(true));
    assert!(!create_called.get());
    assert!(!set_called.get());
}

#[test]
fn vault_preflight_dispatch_keeps_current_create_override_and_base_flags_masks() {
    let create_mask = get_vault_flags_mask_for_txn_type(TxType::VAULT_CREATE, || 0x3ffc_ffff);
    let clawback_mask = get_vault_flags_mask_for_txn_type(TxType::VAULT_CLAWBACK, || {
        panic!("base vault types should not evaluate the create flags mask helper")
    });

    assert_eq!(VAULT_FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
    assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
    assert_eq!(VAULT_BASE_FLAGS_MASK, 0x3fff_ffff);
    assert_eq!(create_mask, Ok(0x3ffc_ffff));
    assert_eq!(clawback_mask, Ok(0x3fff_ffff));
}

#[test]
fn vault_preflight_dispatch_source_wrappers_preserve_vault_subset_unknowns() {
    let tx = TestTx {
        txn_type: TxType::BATCH,
    };

    let extra = run_vault_check_extra_features_for_txn_source(&tx, || true, || true);
    let mask = get_vault_flags_mask_for_txn_source(&tx, || 0x3ffc_ffff);
    let preflight = run_vault_preflight_for_txn_source(
        true,
        &tx,
        || true,
        || true,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(extra, Err(UnknownTransactionType::new(TxType::BATCH)));
    assert_eq!(mask, Err(UnknownTransactionType::new(TxType::BATCH)));
    assert_eq!(preflight, Err(UnknownTransactionType::new(TxType::BATCH)));
}
