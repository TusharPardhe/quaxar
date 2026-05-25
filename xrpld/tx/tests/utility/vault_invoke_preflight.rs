//! Integration tests that pin the vault-family `invokePreflight` shell to the
//! current C++ vault transaction behavior.

use std::cell::RefCell;

use protocol::{SeqProxy, Ter, TxType};
use tx::vault_invoke_preflight::{
    run_vault_invoke_preflight_for_txn_source,
    run_vault_invoke_preflight_for_txn_source_with_consequences,
    run_vault_invoke_preflight_for_txn_type,
    run_vault_invoke_preflight_for_txn_type_with_consequences,
};
use tx::{HasTxnType, TxConsequences, UnknownTransactionType, VaultTxnType};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_invoke_preflight_short_circuits_feature_gate() {
    let result = run_vault_invoke_preflight_for_txn_type(
        false,
        TxType::VAULT_CREATE,
        || panic!("feature gate should skip create extra-features"),
        || panic!("feature gate should skip set extra-features"),
        || panic!("feature gate should skip create flags mask"),
        |_| panic!("feature gate should skip preflight1"),
        || panic!("feature gate should skip create preflight"),
        || panic!("feature gate should skip set preflight"),
        || panic!("feature gate should skip delete preflight"),
        || panic!("feature gate should skip deposit preflight"),
        || panic!("feature gate should skip withdraw preflight"),
        || panic!("feature gate should skip clawback preflight"),
        || panic!("feature gate should skip preflight2"),
    );

    assert_eq!(result, Ok(Ter::TEM_DISABLED));
}

#[test]
fn vault_invoke_preflight_runs_current_for_create_path() {
    let trace = RefCell::new(Vec::new());

    let result = run_vault_invoke_preflight_for_txn_type(
        true,
        TxType::VAULT_CREATE,
        || {
            trace.borrow_mut().push("create-extra");
            true
        },
        || panic!("create path should not run set extra-features"),
        || {
            trace.borrow_mut().push("create-flags");
            0x3ffc_ffff
        },
        |mask| {
            trace.borrow_mut().push("preflight1");
            assert_eq!(mask, 0x3ffc_ffff);
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("create-preflight");
            Ter::TES_SUCCESS
        },
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || {
            trace.borrow_mut().push("preflight2");
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ok(Ter::TES_SUCCESS));
    assert_eq!(
        trace.into_inner(),
        vec![
            "create-extra",
            "create-flags",
            "preflight1",
            "create-preflight",
            "preflight2"
        ]
    );
}

#[test]
fn vault_invoke_preflight_returns_first_failure_unchanged() {
    let preflight1_failure = run_vault_invoke_preflight_for_txn_type(
        true,
        TxType::VAULT_DELETE,
        || true,
        || true,
        || panic!("delete path should not read create flags mask"),
        |_| Ter::TEM_INVALID_FLAG,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("preflight1 failure should skip delete preflight"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("preflight1 failure should skip preflight2"),
    );
    let preflight2_failure = run_vault_invoke_preflight_for_txn_type(
        true,
        TxType::VAULT_CLAWBACK,
        || true,
        || true,
        || panic!("clawback path should not read create flags mask"),
        |_| Ter::TES_SUCCESS,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || Ter::TES_SUCCESS,
        || Ter::TEM_INVALID,
    );

    assert_eq!(preflight1_failure, Ok(Ter::TEM_INVALID_FLAG));
    assert_eq!(preflight2_failure, Ok(Ter::TEM_INVALID));
}

#[test]
fn vault_invoke_preflight_uses_base_flags_mask_for_non_create_vaults() {
    let observed = RefCell::new(None);

    let result = run_vault_invoke_preflight_for_txn_type(
        true,
        TxType::VAULT_SET,
        || true,
        || true,
        || panic!("set path should not read create flags mask"),
        |mask| {
            *observed.borrow_mut() = Some(mask);
            Ter::TES_SUCCESS
        },
        || panic!("wrong vault preflight selected"),
        || Ter::TES_SUCCESS,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ok(Ter::TES_SUCCESS));
    assert_eq!(*observed.borrow(), Some(0x3fff_ffff));
}

#[test]
fn vault_invoke_preflight_source_wrapper_preserves_unknowns_subset() {
    let tx = TestTx {
        txn_type: TxType::BATCH,
    };

    let result = run_vault_invoke_preflight_for_txn_source(
        true,
        &tx,
        || true,
        || true,
        || 0x3ffc_ffff,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Err(UnknownTransactionType::new(TxType::BATCH)));
}

#[test]
fn vault_invoke_preflight_with_consequences_builds_success_consequences_only_on_success() {
    let consequences_called = RefCell::new(false);

    let result = run_vault_invoke_preflight_for_txn_type_with_consequences(
        true,
        TxType::VAULT_CREATE,
        || true,
        || true,
        || 0x3ffc_ffff,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        |vault_txn_type| {
            *consequences_called.borrow_mut() = true;
            assert_eq!(vault_txn_type, VaultTxnType::Create);
            TxConsequences::new(12, SeqProxy::sequence(5))
        },
    );

    assert_eq!(
        result,
        Ok((
            Ter::TES_SUCCESS,
            TxConsequences::new(12, SeqProxy::sequence(5))
        ))
    );
    assert!(*consequences_called.borrow());
}

#[test]
fn vault_invoke_preflight_with_consequences_maps_failure_consequences() {
    let consequences_called = RefCell::new(false);

    let result = run_vault_invoke_preflight_for_txn_type_with_consequences(
        true,
        TxType::VAULT_DELETE,
        || true,
        || true,
        || panic!("delete path should not read create flags mask"),
        |_| Ter::TEM_INVALID_FLAG,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        |_vault_txn_type| {
            *consequences_called.borrow_mut() = true;
            TxConsequences::new(12, SeqProxy::sequence(5))
        },
    );

    assert_eq!(
        result,
        Ok((
            Ter::TEM_INVALID_FLAG,
            TxConsequences::from_preflight_result(Ter::TEM_INVALID_FLAG)
        ))
    );
    assert!(!*consequences_called.borrow());
}

#[test]
fn vault_invoke_preflight_source_with_consequences_uses_txn_type_from_source_subset() {
    let tx = TestTx {
        txn_type: TxType::VAULT_WITHDRAW,
    };

    let result = run_vault_invoke_preflight_for_txn_source_with_consequences(
        true,
        &tx,
        || true,
        || true,
        || 0x3ffc_ffff,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        |vault_txn_type| {
            assert_eq!(vault_txn_type, VaultTxnType::Withdraw);
            TxConsequences::new(7, SeqProxy::ticket(9))
        },
    );

    assert_eq!(
        result,
        Ok((
            Ter::TES_SUCCESS,
            TxConsequences::new(7, SeqProxy::ticket(9))
        ))
    );
}
